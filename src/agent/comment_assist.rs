use chrono::Utc;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::agent::parse::{ci_is_failing, parse_failing_runs, parse_pr_line};
use crate::app::AppEvent;
use crate::config::Config;
use crate::engine::AgentSpec;
use crate::error::{CoworkerError, Result};
use crate::llm::LlmClient;
use crate::mcp::helpers::lazy_tool;
use crate::mcp::McpClient;
use crate::output::digest::{publish_digest, IncrementalDigest};
use crate::store::{Approval, ApprovalKind, ApprovalStatus, Store};

pub struct CommentAssistOutcome {
    pub drafts: u32,
}

impl CommentAssistOutcome {
    pub fn format_summary(&self) -> String {
        format!("comment-assist: {} comment draft(s) queued", self.drafts)
    }
}

pub async fn run_comment_assist(
    config: &Config,
    mcp: &dyn McpClient,
    llm: &LlmClient,
    store: &dyn Store,
    agent: &AgentSpec,
    events: &broadcast::Sender<AppEvent>,
    log: impl Fn(&str, String),
) -> Result<CommentAssistOutcome> {
    if !mcp.is_available() {
        return Err(CoworkerError::Workflow(
            "unistar-mcp is required for comment-assist".into(),
        ));
    }

    let mut digest = IncrementalDigest::begin(agent);
    let mut drafts = 0u32;

    for repo in config.repos.clone() {
        log("info", format!("comment-assist: failing PRs in {repo}"));
        let list = lazy_tool(
            mcp,
            "pr_list_open",
            serde_json::json!({ "repo": repo, "limit": config.policy.max_prs_per_repo }),
        )
        .await?;

        digest.begin_repo(&repo);
        for line in list.lines() {
            let Some(pr) = parse_pr_line(line) else {
                continue;
            };
            if pr.is_draft || !ci_is_failing(&pr.ci) {
                continue;
            }

            let analyze = lazy_tool(
                mcp,
                "ci_analyze_pr_failures",
                serde_json::json!({ "repo": repo, "pr_number": pr.number }),
            )
            .await
            .unwrap_or_else(|_| "CI analysis unavailable".into());

            let mut context = analyze.clone();
            for run in parse_failing_runs(&analyze).into_iter().take(2) {
                if let Ok(summary) = lazy_tool(
                    mcp,
                    "ci_get_run_summary",
                    serde_json::json!({ "repo": repo, "run_id": run.run_id }),
                )
                .await
                {
                    context.push_str(&format!(
                        "\n\nRun {} ({}) summary:\n{summary}",
                        run.run_id, run.workflow
                    ));
                }
            }

            let body = if llm.is_online() {
                let prompt = format!(
                    "Draft a concise, helpful GitHub PR comment for CI failures (markdown, no fluff):\n\
PR #{number} {title}\n\n{context}",
                    number = pr.number,
                    title = pr.title,
                    context = context.chars().take(4000).collect::<String>()
                );
                llm.summarize_plain(&prompt)
                    .await
                    .unwrap_or_else(|_| format!("CI failures detected:\n\n{context}"))
            } else {
                format!(
                    "CI failures detected on this PR:\n\n```\n{}\n```",
                    context.chars().take(1500).collect::<String>()
                )
            };

            let approval = Approval {
                id: Uuid::new_v4(),
                kind: ApprovalKind::PostComment,
                repo: repo.clone(),
                pr_number: Some(pr.number),
                run_id: None,
                target_branch: None,
                incident_id: None,
                description: format!(
                    "Post CI failure comment on #{number} {title}",
                    number = pr.number,
                    title = pr.title
                ),
                status: ApprovalStatus::Pending,
                created_at: Utc::now(),
                decided_at: None,
                comment_body: Some(body.clone()),
            };
            store.push_approval(&approval).await?;
            drafts += 1;
            digest.push_report_line(&format!(
                "Queued comment draft for #{} — approval {}",
                pr.number, approval.id
            ));
        }
    }

    if drafts == 0 {
        digest.push_report_line("_No failing PRs needing comments._");
    }

    let final_digest = digest.finish();
    publish_digest(config, store, events, &final_digest).await?;
    Ok(CommentAssistOutcome { drafts })
}
