use chrono::Utc;
use serde_json::json;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::agent::log_pages::parse_log_page;
use crate::agent::parse::{
    extract_ci_kind, extract_failing_runs_from_overview, parse_failing_runs, ParsedPrLine,
};
use crate::app::AppEvent;
use crate::config::{Config, RuleConfig};
use crate::engine::prompt::compose_classify_prompt;
use crate::engine::SkillSpec;
use crate::error::Result;
use crate::github::helpers::gh_tool;
use crate::github::GithubHarness;
use crate::llm::{
    append_log_chunk, format_policy_digest_line, format_policy_digest_line_from_classify,
    next_prior_summary, ClassifyResult, ClassifyVerdict, LlmClient,
};
use crate::store::{Approval, ApprovalKind, ApprovalStatus, PrSnapshot, Store};

#[derive(Debug, Clone)]
pub struct TriageRunEntry {
    pub verdict: ClassifyVerdict,
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct TriageOutcome {
    /// PR status / analyze preamble (shown once in the first digest bucket).
    pub preamble: Vec<String>,
    /// Classified failing runs, bucketed by verdict in the digest.
    pub runs: Vec<TriageRunEntry>,
    /// Analyze/parse failed — treat whole PR as needs attention (no run split).
    pub fallback_attention: bool,
}

impl TriageOutcome {
    pub fn full_note(&self) -> String {
        let mut parts = self.preamble.clone();
        for run in &self.runs {
            parts.extend(run.lines.clone());
        }
        parts.join("\n")
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn triage_pr(
    config: &Config,
    github: &GithubHarness,
    llm: &LlmClient,
    store: &dyn Store,
    classify_skills: &[SkillSpec],
    repo: &str,
    pr: &ParsedPrLine,
    progress: Option<&broadcast::Sender<AppEvent>>,
) -> Result<TriageOutcome> {
    let pr_number = pr.number;

    let playbook = crate::engine::playbook::few_shot_prefix(store, 3).await;
    let classify_system = compose_classify_prompt(&playbook, classify_skills);

    let overview = gh_tool(
        github,
        "pr_get_overview",
        json!({ "repo": repo, "pr_number": pr_number }),
    )
    .await?;

    let mut outcome = TriageOutcome {
        preamble: vec![overview.lines().next().unwrap_or("PR overview").to_string()],
        ..Default::default()
    };

    let analyze_text = match extract_failing_runs_from_overview(&overview) {
        Some(section) => section,
        None => match gh_tool(
            github,
            "ci_analyze_pr_failures",
            json!({ "repo": repo, "pr_number": pr_number }),
        )
        .await
        {
            Ok(t) => t,
            Err(e) => {
                outcome.preamble.push(format!("CI analyze skipped: {e}"));
                outcome.fallback_attention = ci_needs_attention(&pr.ci);
                save_snapshot(store, repo, pr, &outcome.full_note()).await?;
                return Ok(outcome);
            }
        },
    };

    if matches!(
        extract_ci_kind(&analyze_text),
        Some("external_only") | Some("pending") | Some("clean")
    ) || analyze_text
        .to_ascii_lowercase()
        .contains("no failing github actions")
        || analyze_text.contains("Do not call ci_get_failed_logs for external checks")
    {
        outcome
            .preamble
            .push(analyze_text.lines().next().unwrap_or("").to_string());
        if analyze_text.contains("External checks")
            || extract_ci_kind(&analyze_text) == Some("external_only")
        {
            outcome.preamble.push(
                "External CI failing — inspect PR checks tab; triage cannot fetch external logs."
                    .into(),
            );
        } else if extract_ci_kind(&analyze_text) == Some("pending") {
            outcome
                .preamble
                .push("CI checks still pending — re-triage when complete.".into());
        } else {
            outcome
                .preamble
                .push("External CI may still be failing — check the PR page.".into());
        }
        outcome.fallback_attention = true;
        save_snapshot(store, repo, pr, &outcome.full_note()).await?;
        return Ok(outcome);
    }

    if analyze_text.contains("waiting for approval (action_required") {
        outcome.preamble.push(
            analyze_text
                .lines()
                .find(|l| l.contains("action_required"))
                .unwrap_or("CI waiting for approval")
                .to_string(),
        );
        outcome.preamble.push(
            "Workflow approval gate — not a code failure; human action required on GitHub.".into(),
        );
        outcome.fallback_attention = true;
        save_snapshot(store, repo, pr, &outcome.full_note()).await?;
        return Ok(outcome);
    }

    let runs = parse_failing_runs(&analyze_text);
    if runs.is_empty() {
        outcome
            .preamble
            .push("Could not parse failing runs from analyze output.".into());
        outcome.fallback_attention = true;
        save_snapshot(store, repo, pr, &outcome.full_note()).await?;
        return Ok(outcome);
    }

    let mut tool_calls = 0u32;
    for run in runs {
        if tool_calls >= config.policy.max_tool_calls_per_pr {
            outcome
                .preamble
                .push("(tool call budget exhausted for this PR)".into());
            break;
        }

        if run.conclusion == "action_required" {
            outcome.runs.push(TriageRunEntry {
                verdict: ClassifyVerdict::Policy,
                lines: vec![format_policy_digest_line(
                    repo,
                    run.run_id,
                    &run.workflow,
                    "needs approval",
                )],
            });
            continue;
        }

        if tool_calls < config.policy.max_tool_calls_per_pr {
            if let Ok(summary) = gh_tool(
                github,
                "ci_get_run_summary",
                json!({ "repo": repo, "run_id": run.run_id }),
            )
            .await
            {
                tool_calls += 1;
                outcome.preamble.push(format!(
                    "Run {} ({}) summary:\n{summary}",
                    run.run_id, run.workflow
                ));
            }
        }

        let mut combined_logs = String::new();
        let mut classify = None;

        if tool_calls < config.policy.max_tool_calls_per_pr {
            emit_triage_status(
                progress,
                format_triage_digest_status(repo, pr_number, run.run_id),
            );
            if let Ok(digest) = gh_tool(
                github,
                "ci_get_failure_digest",
                json!({ "repo": repo, "run_id": run.run_id }),
            )
            .await
            {
                tool_calls += 1;
                outcome.preamble.push(format!(
                    "Run {} ({}) digest:\n{digest}",
                    run.run_id, run.workflow
                ));
                if let Some(excerpt) = digest_excerpt(&digest) {
                    combined_logs = excerpt;
                }
                classify = classify_from_failure_digest(&digest, &run.workflow, &config.rules);
            }
        }

        if classify.is_none() {
            let page_lines = config.llm.log_page_lines.max(1);
            let max_pages = config.llm.max_log_pages.max(1).min(
                config
                    .policy
                    .max_tool_calls_per_pr
                    .saturating_sub(tool_calls),
            );

            let mut offset = 0u32;
            let mut prior_summary = String::new();

            for page_num in 1..=max_pages {
                if tool_calls >= config.policy.max_tool_calls_per_pr {
                    outcome
                        .preamble
                        .push("(tool call budget exhausted for this PR)".into());
                    break;
                }

                emit_triage_status(
                    progress,
                    format_triage_log_page_status(repo, pr_number, page_num, max_pages, run.run_id),
                );

                let resp = gh_tool(
                    github,
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

                if classify.is_none() {
                    if let Some(rule_match) =
                        crate::engine::rules::apply_rules(&config.rules, &run.workflow, &page.body)
                    {
                        use crate::engine::rules::{verdict_from_rule, RuleMatch};
                        classify = Some(ClassifyResult {
                            verdict: verdict_from_rule(rule_match),
                            reason: format!("Matched YAML rule ({rule_match:?})"),
                            diagnosis: None,
                            recommended_action: if rule_match == RuleMatch::SuggestRerun {
                                Some("Suggest rerunning the failed workflow".into())
                            } else {
                                None
                            },
                            test_name: None,
                            used_llm: false,
                            pages_read: page_num,
                            page_summary: None,
                        });
                        if rule_match == RuleMatch::SkipLlm {
                            break;
                        }
                    }
                }

                if classify.is_some() {
                    break;
                }

                let result = llm
                    .classify_log_page(
                        &classify_system,
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
        }

        let classify = match classify {
            Some(c) => c,
            None => {
                outcome.runs.push(TriageRunEntry {
                    verdict: ClassifyVerdict::Unknown,
                    lines: vec![format!(
                        "- run {} {} → skipped (digest inconclusive, no log pages fetched)",
                        run.run_id, run.workflow
                    )],
                });
                continue;
            }
        };

        let _logs = combined_logs;

        if matches!(classify.verdict, ClassifyVerdict::Policy) {
            outcome.runs.push(TriageRunEntry {
                verdict: ClassifyVerdict::Policy,
                lines: vec![format_policy_digest_line_from_classify(
                    repo,
                    run.run_id,
                    &run.workflow,
                    &classify,
                )],
            });
            continue;
        }

        let flaky_for_rerun = matches!(classify.verdict, ClassifyVerdict::Flaky)
            || (matches!(classify.verdict, ClassifyVerdict::Unknown)
                && !config.flaky.record_real_bugs);

        if flaky_for_rerun && !config.policy.auto_rerun_flaky {
            store
                .push_approval(&Approval {
                    id: Uuid::new_v4(),
                    kind: ApprovalKind::RerunFlaky,
                    repo: repo.to_string(),
                    pr_number: Some(pr_number),
                    run_id: Some(run.run_id),
                    target_branch: None,
                    incident_id: None,
                    description: format!(
                        "Flaky CI on PR #{pr_number} run {} ({}) — approve rerun?",
                        run.run_id, run.workflow
                    ),
                    status: ApprovalStatus::Pending,
                    created_at: Utc::now(),
                    decided_at: None,
                    comment_body: None,
                    issue_number: None,
                    label: None,
                })
                .await?;
        }

        outcome.runs.push(TriageRunEntry {
            verdict: classify.verdict,
            lines: crate::llm::format_classify_digest_lines(
                repo,
                run.run_id,
                &run.workflow,
                &classify,
            ),
        });
    }

    save_snapshot(store, repo, pr, &outcome.full_note()).await?;

    if !outcome.runs.is_empty() {
        let verdict = outcome
            .runs
            .iter()
            .map(|r| format!("{:?}", r.verdict))
            .collect::<Vec<_>>()
            .join(",");
        let transcript = crate::engine::playbook::transcript_from_triage(
            repo,
            pr_number,
            "triage",
            &verdict,
            &outcome.full_note(),
        );
        let _ = store.save_transcript(&transcript).await;
    }

    Ok(outcome)
}

async fn save_snapshot(store: &dyn Store, repo: &str, pr: &ParsedPrLine, note: &str) -> Result<()> {
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

fn emit_triage_status(progress: Option<&broadcast::Sender<AppEvent>>, msg: impl Into<String>) {
    if let Some(tx) = progress {
        let _ = tx.send(AppEvent::StatusMessage(msg.into()));
    }
}

pub fn format_triage_log_page_status(
    repo: &str,
    pr_number: u32,
    page_num: u32,
    max_pages: u32,
    run_id: i64,
) -> String {
    format!(
        "triage {repo}#{pr_number}: ci_get_failed_logs page {page_num}/{max_pages} (run {run_id})"
    )
}

pub fn format_triage_digest_status(repo: &str, pr_number: u32, run_id: i64) -> String {
    format!("triage {repo}#{pr_number}: ci_get_failure_digest (run {run_id})")
}

fn digest_excerpt(digest: &str) -> Option<String> {
    let marker = "Excerpt:";
    let start = digest.find(marker)? + marker.len();
    let rest = &digest[start..];
    let end = rest.find("\nNext:").unwrap_or(rest.len());
    let excerpt = rest[..end].trim();
    if excerpt.is_empty() {
        None
    } else {
        Some(excerpt.to_string())
    }
}

fn digest_test_name(digest: &str) -> Option<String> {
    digest
        .lines()
        .find_map(|l| l.strip_prefix("Test:").map(|s| s.trim().to_string()))
        .filter(|s| !s.is_empty())
}

fn classify_from_failure_digest(
    digest: &str,
    workflow: &str,
    rules: &[RuleConfig],
) -> Option<ClassifyResult> {
    let excerpt = digest_excerpt(digest).unwrap_or_default();
    let corpus = if excerpt.is_empty() {
        digest.to_string()
    } else {
        format!("{digest}\n{excerpt}")
    };

    if let Some(rule_match) = crate::engine::rules::apply_rules(rules, workflow, &corpus) {
        use crate::engine::rules::{verdict_from_rule, RuleMatch};
        return Some(ClassifyResult {
            verdict: verdict_from_rule(rule_match),
            reason: format!("Digest matched YAML rule ({rule_match:?})"),
            diagnosis: None,
            recommended_action: if rule_match == RuleMatch::SuggestRerun {
                Some("Suggest rerunning the failed workflow".into())
            } else {
                None
            },
            test_name: digest_test_name(digest),
            used_llm: false,
            pages_read: 0,
            page_summary: None,
        });
    }

    let verdict_line = digest.lines().find(|l| l.starts_with("Verdict:"))?;
    let verdict = verdict_line.strip_prefix("Verdict:")?.trim();
    let verdict = verdict.split_whitespace().next()?;

    let classify_verdict = match verdict {
        "timeout" | "infra" => ClassifyVerdict::Flaky,
        "test" | "auth" | "external_ci" => ClassifyVerdict::Real,
        _ => return None,
    };

    Some(ClassifyResult {
        verdict: classify_verdict,
        reason: format!("ci_get_failure_digest policy verdict: {verdict}"),
        diagnosis: None,
        recommended_action: None,
        test_name: digest_test_name(digest),
        used_llm: false,
        pages_read: 0,
        page_summary: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::ClassifyVerdict;

    #[test]
    fn triage_log_page_status_format() {
        let msg = format_triage_log_page_status("acme/widget", 42, 2, 6, 12345);
        assert!(msg.contains("page 2/6"));
        assert!(msg.contains("run 12345"));
    }

    #[test]
    fn classify_from_digest_timeout_is_flaky() {
        let digest = "Run 1 CI\nVerdict: timeout (timeout)\n\nExcerpt:\ncontext deadline exceeded\n\nNext: ci_get_failed_logs";
        let c = classify_from_failure_digest(digest, "CI", &[]).unwrap();
        assert_eq!(c.verdict, ClassifyVerdict::Flaky);
        assert!(!c.used_llm);
    }

    #[test]
    fn classify_from_digest_unknown_needs_logs() {
        let digest = "Run 1 CI\nVerdict: unknown (no_rule_match)\n\nExcerpt:\nsomething odd\n\nNext: ci_get_failed_logs";
        assert!(classify_from_failure_digest(digest, "CI", &[]).is_none());
    }
}
