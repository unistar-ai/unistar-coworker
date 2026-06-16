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

pub struct LightReviewOutcome {
    pub prs_reviewed: u32,
}

impl LightReviewOutcome {
    pub fn format_summary(&self) -> String {
        format!("light-review: {} PR(s) reviewed", self.prs_reviewed)
    }
}

fn split_diff_files(diff: &str) -> Vec<(String, String)> {
    let mut chunks = Vec::new();
    let mut current_path = String::new();
    let mut current_body = String::new();
    for line in diff.lines() {
        if line.starts_with("diff --git ") {
            if !current_path.is_empty() {
                chunks.push((current_path.clone(), current_body.clone()));
            }
            current_path = line.to_string();
            current_body.clear();
        } else {
            current_body.push_str(line);
            current_body.push('\n');
        }
    }
    if !current_path.is_empty() {
        chunks.push((current_path, current_body));
    }
    chunks
}

pub async fn run_light_review(
    config: &Config,
    mcp: &dyn McpClient,
    llm: &LlmClient,
    store: &dyn Store,
    agent: &AgentSpec,
    events: &broadcast::Sender<AppEvent>,
    log: impl Fn(&str, String),
) -> Result<LightReviewOutcome> {
    if !mcp.is_available() {
        return Err(CoworkerError::Workflow(
            "unistar-mcp is required for light-review".into(),
        ));
    }

    let mut digest = IncrementalDigest::begin(agent);
    let mut reviewed = 0u32;

    'repos: for repo in config.repos.clone() {
        let list = lazy_tool(
            mcp,
            "pr_list_open",
            serde_json::json!({ "repo": repo, "limit": 5 }),
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

            log(
                "info",
                format!("light-review: diff map-reduce for {repo}#{}", pr.number),
            );
            let diff = lazy_tool(
                mcp,
                "pr_get_diff",
                serde_json::json!({ "repo": repo, "pr_number": pr.number, "max_bytes": 48000 }),
            )
            .await?;

            let files = split_diff_files(&diff);
            let mut findings = Vec::new();
            for (path, chunk) in files.iter().take(12) {
                if chunk.len() < 20 {
                    continue;
                }
                let excerpt = chunk.chars().take(6000).collect::<String>();
                if llm.is_online() {
                    let prompt = format!(
                        "File-level risk review (bullet risks only, no line comments):\n{path}\n\n{excerpt}"
                    );
                    if let Ok(note) = llm.summarize_plain(&prompt).await {
                        if !note.is_empty() {
                            findings.push(format!("**{path}**: {note}"));
                        }
                    }
                } else {
                    findings.push(format!("**{path}**: {} lines changed", chunk.lines().count()));
                }
            }

            reviewed += 1;
            digest.push_report_line(&format!("### #{number} {title}", number = pr.number, title = pr.title));
            if findings.is_empty() {
                digest.push_report_line("_No notable risks flagged._");
            } else {
                for f in findings {
                    digest.push_report_line(&f);
                }
            }
            if reviewed >= 3 {
                break 'repos;
            }
        }
    }

    let final_digest = digest.finish();
    publish_digest(config, store, events, &final_digest).await?;
    Ok(LightReviewOutcome { prs_reviewed: reviewed })
}
