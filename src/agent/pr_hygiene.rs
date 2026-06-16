use tokio::sync::broadcast;

use crate::agent::parse::parse_pr_line;
use crate::app::AppEvent;
use crate::config::Config;
use crate::engine::AgentSpec;
use crate::error::{CoworkerError, Result};
use crate::mcp::helpers::lazy_tool;
use crate::mcp::McpClient;
use crate::output::digest::{publish_digest, IncrementalDigest};
use crate::store::Store;

pub struct PrHygieneOutcome {
    pub findings: u32,
}

impl PrHygieneOutcome {
    pub fn format_summary(&self) -> String {
        format!("pr-hygiene: {} finding(s)", self.findings)
    }
}

fn is_docs_only(files_text: &str) -> bool {
    let mut paths = Vec::new();
    for line in files_text.lines().skip(1) {
        let path = line.split_whitespace().next().unwrap_or("");
        if !path.is_empty() && !path.starts_with("totals:") {
            paths.push(path);
        }
    }
    !paths.is_empty() && paths.iter().all(|p| {
        p.ends_with(".md")
            || p.ends_with(".rst")
            || p.starts_with("docs/")
            || p.contains("/docs/")
    })
}

fn total_line_delta(files_text: &str) -> u32 {
    for l in files_text.lines() {
        if let Some(rest) = l.strip_prefix("totals: +") {
            if let Some((add_s, del_part)) = rest.split_once("/-") {
                if let (Ok(add), Ok(del)) = (add_s.parse::<u32>(), del_part.parse::<u32>()) {
                    return add + del;
                }
            }
        }
    }
    0
}

pub async fn run_pr_hygiene(
    config: &Config,
    mcp: &dyn McpClient,
    store: &dyn Store,
    agent: &AgentSpec,
    events: &broadcast::Sender<AppEvent>,
    log: impl Fn(&str, String),
) -> Result<PrHygieneOutcome> {
    if !mcp.is_available() {
        return Err(CoworkerError::Workflow(
            "unistar-mcp is required for pr-hygiene".into(),
        ));
    }

    let days = config.hygiene.stale_days;
    let large_threshold = config.hygiene.large_pr_lines;
    let mut digest = IncrementalDigest::begin(agent);
    let mut findings = 0u32;

    for repo in config.repos.clone() {
        log("info", format!("pr-hygiene: stale scan for {repo}"));
        let stale = lazy_tool(
            mcp,
            "pr_list_stale",
            serde_json::json!({ "repo": repo, "days": days, "limit": 15 }),
        )
        .await?;

        digest.begin_repo(&repo);
        if stale.contains("No stale open PRs") {
            digest.push_report_line("_No stale PRs._");
            continue;
        }

        for line in stale.lines().skip(1) {
            let Some(pr) = parse_pr_line(line) else {
                continue;
            };
            findings += 1;
            digest.push_report_line(&format!("**Stale** #{number} {title}", number = pr.number, title = pr.title));

            let files = lazy_tool(
                mcp,
                "pr_list_changed_files",
                serde_json::json!({ "repo": repo, "pr_number": pr.number }),
            )
            .await
            .unwrap_or_default();

            if is_docs_only(&files) {
                findings += 1;
                digest.push_report_line(&format!("  → docs-only PR #{number}", number = pr.number));
            }
            let delta = total_line_delta(&files);
            if delta >= large_threshold {
                findings += 1;
                digest.push_report_line(&format!(
                    "  → **large PR** #{number} (+/- {delta} lines)",
                    number = pr.number
                ));
            }
        }
    }

    let final_digest = digest.finish();
    publish_digest(config, store, events, &final_digest).await?;
    Ok(PrHygieneOutcome { findings })
}
