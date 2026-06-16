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

const BREAKING_PATH_PATTERNS: &[&str] = &[
    "/api/",
    "migration",
    "CHANGELOG",
    "breaking",
    "proto/",
    "openapi",
    "schema/",
];

pub struct BreakingSniffOutcome {
    pub hits: u32,
}

impl BreakingSniffOutcome {
    pub fn format_summary(&self) -> String {
        format!("breaking-sniff: {} potential breaking change(s)", self.hits)
    }
}

fn path_rule_hit(path: &str) -> Option<&'static str> {
    let lower = path.to_ascii_lowercase();
    BREAKING_PATH_PATTERNS
        .iter()
        .find(|pat| lower.contains(**pat))
        .copied()
}

pub async fn run_breaking_sniff(
    config: &Config,
    mcp: &dyn McpClient,
    llm: &LlmClient,
    store: &dyn Store,
    agent: &AgentSpec,
    events: &broadcast::Sender<AppEvent>,
    log: impl Fn(&str, String),
) -> Result<BreakingSniffOutcome> {
    if !mcp.is_available() {
        return Err(CoworkerError::Workflow(
            "unistar-mcp is required for breaking-sniff".into(),
        ));
    }

    let mut digest = IncrementalDigest::begin(agent);
    let mut hits = 0u32;

    for repo in config.repos.clone() {
        log("info", format!("breaking-sniff: open PRs in {repo}"));
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
            if pr.is_draft {
                continue;
            }

            let files = lazy_tool(
                mcp,
                "pr_list_changed_files",
                serde_json::json!({ "repo": repo, "pr_number": pr.number }),
            )
            .await?;

            let mut rule_hits = Vec::new();
            for fline in files.lines().skip(1) {
                let path = fline.split_whitespace().next().unwrap_or("");
                if let Some(rule) = path_rule_hit(path) {
                    rule_hits.push(format!("{path} (rule: {rule})"));
                }
            }

            if rule_hits.is_empty() {
                continue;
            }

            hits += 1;
            digest.push_report_line(&format!("### #{number} {title}", number = pr.number, title = pr.title));
            for h in &rule_hits {
                digest.push_report_line(&format!("- {h}"));
            }

            if llm.is_online() {
                let diff = lazy_tool(
                    mcp,
                    "pr_get_diff",
                    serde_json::json!({ "repo": repo, "pr_number": pr.number, "max_bytes": 32000 }),
                )
                .await
                .unwrap_or_default();
                let prompt = format!(
                    "Summarize potential breaking changes (1-3 bullets) for PR #{number}:\n{diff}",
                    number = pr.number,
                    diff = diff.chars().take(8000).collect::<String>()
                );
                if let Ok(note) = llm.summarize_plain(&prompt).await {
                    if !note.is_empty() {
                        digest.push_report_line(&format!("_LLM:_ {note}"));
                    }
                }
            }
        }
    }

    if hits == 0 {
        digest.push_report_line("_No breaking-change path rules matched._");
    }

    let final_digest = digest.finish();
    publish_digest(config, store, events, &final_digest).await?;
    Ok(BreakingSniffOutcome { hits })
}
