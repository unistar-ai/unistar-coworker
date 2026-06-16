use tokio::sync::broadcast;

use crate::app::AppEvent;
use crate::config::Config;
use crate::engine::AgentSpec;
use crate::error::{CoworkerError, Result};
use crate::mcp::helpers::lazy_tool;
use crate::mcp::McpClient;
use crate::output::digest::{publish_digest, IncrementalDigest};
use crate::store::Store;

pub struct SecurityDigestOutcome {
    pub alerts: u32,
}

impl SecurityDigestOutcome {
    pub fn format_summary(&self) -> String {
        format!("security-digest: {} open alert(s) indexed", self.alerts)
    }
}

pub async fn run_security_digest(
    config: &Config,
    mcp: &dyn McpClient,
    store: &dyn Store,
    agent: &AgentSpec,
    events: &broadcast::Sender<AppEvent>,
    log: impl Fn(&str, String),
) -> Result<SecurityDigestOutcome> {
    if !mcp.is_available() {
        return Err(CoworkerError::Workflow(
            "unistar-mcp is required for security-digest".into(),
        ));
    }

    let mut digest = IncrementalDigest::begin(agent);
    let mut total = 0u32;

    for repo in config.repos.clone() {
        log("info", format!("fetching dependabot alerts for {repo}"));
        let text = lazy_tool(
            mcp,
            "alert_list_open",
            serde_json::json!({ "repo": repo, "limit": 20 }),
        )
        .await?;

        digest.begin_repo(&repo);
        if text.contains("No open Dependabot") {
            digest.push_report_line("_No open Dependabot alerts._");
            continue;
        }

        for line in text.lines().skip(1) {
            if line.trim().is_empty() || line.contains("open Dependabot") {
                continue;
            }
            total += 1;
            digest.push_report_line(line.trim());
        }
    }

    if total == 0 {
        digest.push_report_line("_No critical security alerts across repos._");
    }

    let final_digest = digest.finish();
    publish_digest(config, store, events, &final_digest).await?;

    Ok(SecurityDigestOutcome { alerts: total })
}
