use tokio::sync::broadcast;

use crate::agent::parse::parse_pr_line;
use crate::app::AppEvent;
use crate::config::Config;
use crate::engine::AgentSpec;
use crate::error::{CoworkerError, Result};
use crate::llm::LlmClient;
use crate::mcp::helpers::lazy_tool;
use crate::mcp::McpClient;
use crate::output::digest::{publish_digest, IncrementalDigest};
use crate::store::Store;

pub struct ReleaseNotesOutcome {
    pub merged_count: u32,
}

impl ReleaseNotesOutcome {
    pub fn format_summary(&self) -> String {
        format!("release-notes: {} merged PR(s) indexed", self.merged_count)
    }
}

pub async fn run_release_notes(
    config: &Config,
    mcp: &dyn McpClient,
    llm: &LlmClient,
    store: &dyn Store,
    agent: &AgentSpec,
    events: &broadcast::Sender<AppEvent>,
    log: impl Fn(&str, String),
) -> Result<ReleaseNotesOutcome> {
    if !mcp.is_available() {
        return Err(CoworkerError::Workflow(
            "unistar-mcp is required for release-notes".into(),
        ));
    }

    let since_days = config.release.lookback_limit.min(90);
    let mut digest = IncrementalDigest::begin(agent);
    let mut merged_count = 0u32;
    let mut batch_lines = Vec::new();

    for repo in config.repos.clone() {
        log("info", format!("release-notes: merged PRs for {repo}"));
        let text = lazy_tool(
            mcp,
            "pr_list_merged",
            serde_json::json!({
                "repo": repo,
                "since": since_days.to_string(),
                "limit": 40,
            }),
        )
        .await?;

        digest.begin_repo(&repo);
        if text.contains("No merged PRs") {
            digest.push_report_line("_No merged PRs in lookback window._");
            continue;
        }

        for line in text.lines().skip(1) {
            let Some(pr) = parse_pr_line(line) else {
                continue;
            };
            merged_count += 1;
            batch_lines.push(format!("#{} {} (@{})", pr.number, pr.title, pr.author));
            digest.push_report_line(&format!("#{} {}", pr.number, pr.title));
        }
    }

    if !batch_lines.is_empty() && llm.is_online() {
        let prompt = format!(
            "Summarize these merged PRs into a release-notes draft (group by theme, bullet list):\n\n{}",
            batch_lines.join("\n")
        );
        if let Ok(summary) = llm.summarize_plain(&prompt).await {
            digest.push_report_line(&format!("\n### LLM draft\n\n{summary}"));
        }
    }

    let final_digest = digest.finish();
    publish_digest(config, store, events, &final_digest).await?;
    Ok(ReleaseNotesOutcome { merged_count })
}
