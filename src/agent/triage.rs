use chrono::Utc;
use serde_json::json;
use uuid::Uuid;

use crate::agent::log_pages::parse_log_page;
use crate::agent::parse::{parse_failing_runs, ParsedPrLine};
use crate::config::Config;
use crate::engine::Skill;
use crate::error::Result;
use crate::llm::{
    append_log_chunk, format_classify_digest_lines, llm_reason_text, next_prior_summary,
    ClassifyVerdict, LlmClient,
};
use crate::mcp::helpers::lazy_tool;
use crate::mcp::McpClient;
use crate::store::{
    compute_fingerprint, Approval, ApprovalKind, ApprovalStatus, Classification, FlakyIncident,
    PrSnapshot, Store,
};

#[derive(Debug, Clone)]
pub struct TriageOutcome {
    pub note: String,
    pub flaky: bool,
    pub real: bool,
}

pub async fn triage_pr(
    config: &Config,
    mcp: &dyn McpClient,
    llm: &LlmClient,
    store: &dyn Store,
    skill: &Skill,
    repo: &str,
    pr: &ParsedPrLine,
) -> Result<TriageOutcome> {
    let pr_number = pr.number;

    let status = lazy_tool(
        mcp,
        "pr_get_status",
        json!({ "repo": repo, "pr_number": pr_number }),
    )
    .await?;

    let analyze = lazy_tool(
        mcp,
        "ci_analyze_pr_failures",
        json!({ "repo": repo, "pr_number": pr_number }),
    )
    .await;

    let mut notes = vec![status.lines().next().unwrap_or("PR status").to_string()];
    let mut flaky = false;
    let mut real = false;

    let analyze_text = match analyze {
        Ok(t) => t,
        Err(e) => {
            notes.push(format!("CI analyze skipped: {e}"));
            let note = notes.join("\n");
            save_snapshot(store, repo, pr, &note).await?;
            return Ok(TriageOutcome {
                note,
                flaky: false,
                real: ci_needs_attention(&pr.ci),
            });
        }
    };

    if analyze_text.to_ascii_lowercase().contains("no failing github actions") {
        notes.push(analyze_text.lines().next().unwrap_or("").to_string());
        notes.push("External CI may still be failing — check the PR page.".into());
        let note = notes.join("\n");
        save_snapshot(store, repo, pr, &note).await?;
        return Ok(TriageOutcome {
            note,
            flaky: false,
            real: true,
        });
    }

    let runs = parse_failing_runs(&analyze_text);
    if runs.is_empty() {
        notes.push("Could not parse failing runs from analyze output.".into());
        let note = notes.join("\n");
        save_snapshot(store, repo, pr, &note).await?;
        return Ok(TriageOutcome {
            note,
            flaky: false,
            real: true,
        });
    }

    let mut tool_calls = 0u32;
    for run in runs {
        if tool_calls >= config.policy.max_tool_calls_per_pr {
            notes.push("(tool call budget exhausted for this PR)".into());
            break;
        }

        if run.conclusion == "action_required" {
            notes.push(format!(
                "Run {} ({}) needs approval — not a code failure.",
                run.run_id, run.workflow
            ));
            continue;
        }

        let page_lines = config.llm.log_page_lines.max(1);
        let max_pages = config
            .llm
            .max_log_pages
            .max(1)
            .min(config.policy.max_tool_calls_per_pr.saturating_sub(tool_calls));

        let mut offset = 0u32;
        let mut combined_logs = String::new();
        let mut prior_summary = String::new();
        let mut classify = None;

        for page_num in 1..=max_pages {
            if tool_calls >= config.policy.max_tool_calls_per_pr {
                notes.push("(tool call budget exhausted for this PR)".into());
                break;
            }

            let resp = lazy_tool(
                mcp,
                "ci_get_failed_logs",
                json!({
                    "repo": repo,
                    "run_id": run.run_id,
                    "offset_lines": offset,
                    "max_lines": page_lines,
                }),
            )
            .await?;
            tool_calls += 1;

            let page = parse_log_page(&resp);
            append_log_chunk(&mut combined_logs, &page.body);

            let result = llm
                .classify_log_page(
                    &skill.body,
                    repo,
                    pr_number,
                    &run.workflow,
                    &page.body,
                    &combined_logs,
                    &prior_summary,
                    page_num,
                    max_pages,
                )
                .await?;

            if result.verdict != ClassifyVerdict::Unknown {
                classify = Some(result);
                break;
            }

            prior_summary = next_prior_summary(&prior_summary, page_num, &result);

            if !page.has_more {
                classify = Some(result);
                break;
            }
            offset = page.next_offset_lines;
        }

        let classify = match classify {
            Some(c) => c,
            None => {
                notes.push(format!(
                    "- run {} {} → skipped (no log pages fetched)",
                    run.run_id, run.workflow
                ));
                continue;
            }
        };

        let logs = combined_logs;

        let error_sig = logs
            .lines()
            .find(|l| {
                let t = l.to_ascii_lowercase();
                t.contains("error") || t.contains("fail") || t.contains("panic")
            })
            .unwrap_or(logs.as_str())
            .chars()
            .take(200)
            .collect::<String>();

        let fingerprint = compute_fingerprint(
            repo,
            &run.workflow,
            None,
            classify.test_name.as_deref(),
            &error_sig,
        );

        let classification = match classify.verdict {
            ClassifyVerdict::Flaky => {
                flaky = true;
                Classification::LlmFlaky
            }
            ClassifyVerdict::Real => {
                real = true;
                Classification::LlmReal
            }
            ClassifyVerdict::Policy => {
                real = true;
                notes.extend(format_classify_digest_lines(
                    run.run_id,
                    &run.workflow,
                    &classify,
                ));
                continue;
            }
            ClassifyVerdict::Unknown => {
                real = true;
                if config.flaky.record_real_bugs {
                    Classification::LlmReal
                } else {
                    Classification::LlmFlaky
                }
            }
        };

        if matches!(classification, Classification::LlmFlaky)
            || (config.flaky.record_real_bugs && matches!(classification, Classification::LlmReal))
        {
            let incident = FlakyIncident {
                id: Uuid::new_v4(),
                ts: Utc::now(),
                repo: repo.to_string(),
                pr_number: Some(pr_number),
                run_id: run.run_id,
                workflow: run.workflow.clone(),
                job: None,
                step: None,
                test_name: classify.test_name.clone(),
                fingerprint,
                classification,
                log_excerpt: logs.chars().take(2000).collect(),
                llm_reason: Some(llm_reason_text(&classify)),
                rerun_outcome: None,
            };
            let incident_id = incident.id;
            store.record_flaky_incident(&incident).await?;

            if matches!(classification, Classification::LlmFlaky) && !config.policy.auto_rerun_flaky
            {
                store
                    .push_approval(&Approval {
                        id: Uuid::new_v4(),
                        kind: ApprovalKind::RerunFlaky,
                        repo: repo.to_string(),
                        pr_number: Some(pr_number),
                        run_id: Some(run.run_id),
                        target_branch: None,
                        incident_id: Some(incident_id),
                        description: format!(
                            "Flaky CI on PR #{pr_number} run {} ({}) — approve rerun?",
                            run.run_id, run.workflow
                        ),
                        status: ApprovalStatus::Pending,
                        created_at: Utc::now(),
                        decided_at: None,
                    })
                    .await?;
            }
        }

        notes.extend(format_classify_digest_lines(
            run.run_id,
            &run.workflow,
            &classify,
        ));
    }

    let note = notes.join("\n");
    save_snapshot(store, repo, pr, &note).await?;

    Ok(TriageOutcome {
        note,
        flaky,
        real,
    })
}

async fn save_snapshot(
    store: &dyn Store,
    repo: &str,
    pr: &ParsedPrLine,
    note: &str,
) -> Result<()> {
    store
        .upsert_pr_snapshot(&PrSnapshot {
            repo: repo.to_string(),
            number: pr.number,
            title: pr.title.clone(),
            author: pr.author.clone(),
            ci_summary: pr.ci.clone(),
            review_summary: pr.review.clone(),
            is_draft: pr.is_draft,
            fetched_at: Utc::now(),
            triage_note: Some(note.to_string()),
        })
        .await
}

fn ci_needs_attention(ci: &str) -> bool {
    let c = ci.to_ascii_lowercase();
    c.starts_with("failing") || c.contains("fail")
}
