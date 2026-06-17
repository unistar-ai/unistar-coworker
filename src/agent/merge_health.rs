use tokio::sync::broadcast;

use crate::agent::parse::{ci_is_passing, parse_pr_line};
use crate::app::AppEvent;
use crate::config::Config;
use crate::engine::AgentSpec;
use crate::error::{CoworkerError, Result};
use crate::mcp::helpers::lazy_tool;
use crate::mcp::McpClient;
use crate::output::digest::{publish_digest, IncrementalDigest};
use crate::store::Store;

pub struct MergeHealthOutcome {
    pub blocked: u32,
}

impl MergeHealthOutcome {
    pub fn format_summary(&self) -> String {
        format!("merge-health: {} blocked merge candidate(s)", self.blocked)
    }
}

pub async fn run_merge_health(
    config: &Config,
    mcp: &dyn McpClient,
    store: &dyn Store,
    agent: &AgentSpec,
    events: &broadcast::Sender<AppEvent>,
    log: impl Fn(&str, String),
) -> Result<MergeHealthOutcome> {
    if !mcp.is_available() {
        return Err(CoworkerError::Workflow(
            "unistar-mcp is required for merge-health".into(),
        ));
    }

    let mut digest = IncrementalDigest::begin(agent);
    let mut blocked = 0u32;

    for repo in config.repos.clone() {
        log("info", format!("merge-health: scanning {repo}"));
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
            if pr.is_draft || !ci_is_passing(&pr.ci) {
                continue;
            }

            let blockers = lazy_tool(
                mcp,
                "pr_get_merge_blockers",
                serde_json::json!({ "repo": repo, "pr_number": pr.number }),
            )
            .await?;

            if blockers.contains("Mergeable: yes") {
                continue;
            }

            blocked += 1;
            let reason = crate::agent::parse::merge_blockers_summary(&blockers);
            let detail = if reason.is_empty() {
                blockers
                    .lines()
                    .find(|l| l.starts_with("Mergeable:"))
                    .unwrap_or("Mergeable: unknown")
                    .to_string()
            } else {
                reason
            };
            digest.push_report_line(&format!("#{} {} — {}", pr.number, pr.title, detail));
        }
    }

    if blocked == 0 {
        digest.push_report_line("_No theoretically-ready PRs blocked on merge gates._");
    }

    let final_digest = digest.finish();
    publish_digest(config, store, events, &final_digest).await?;
    Ok(MergeHealthOutcome { blocked })
}
