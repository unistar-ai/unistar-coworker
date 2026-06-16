use chrono::Utc;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::agent::parse::parse_pr_line;
use crate::app::AppEvent;
use crate::config::Config;
use crate::engine::AgentSpec;
use crate::error::{CoworkerError, Result};
use crate::llm::LlmClient;
use crate::mcp::helpers::lazy_tool;
use crate::mcp::McpClient;
use crate::output::digest::{publish_digest, IncrementalDigest};
use crate::store::{FlakyQuery, RegressionLink, Store};

pub struct RegressionLinkOutcome {
    pub links: u32,
}

impl RegressionLinkOutcome {
    pub fn format_summary(&self) -> String {
        format!("regression-link: {} correlation(s) saved", self.links)
    }
}

pub async fn run_regression_link(
    config: &Config,
    mcp: &dyn McpClient,
    llm: &LlmClient,
    store: &dyn Store,
    agent: &AgentSpec,
    events: &broadcast::Sender<AppEvent>,
    log: impl Fn(&str, String),
) -> Result<RegressionLinkOutcome> {
    if !mcp.is_available() {
        return Err(CoworkerError::Workflow(
            "unistar-mcp is required for regression-link".into(),
        ));
    }

    let flaky = store
        .list_flaky_tests(FlakyQuery {
            repo: None,
            since_days: Some(7),
            limit: 10,
        })
        .await?;

    let mut digest = IncrementalDigest::begin(agent);
    let mut links = 0u32;

    for test in flaky {
        log(
            "info",
            format!(
                "regression-link: correlating {} in {}",
                test.test_name.as_deref().unwrap_or("?"),
                test.repo
            ),
        );

        let merged = lazy_tool(
            mcp,
            "pr_list_merged",
            serde_json::json!({ "repo": test.repo, "since": "7", "limit": 20 }),
        )
        .await?;

        let pr_lines: Vec<String> = merged
            .lines()
            .skip(1)
            .filter_map(|l| parse_pr_line(l).map(|p| format!("#{} {}", p.number, p.title)))
            .collect();

        if pr_lines.is_empty() {
            continue;
        }

        let summary = if llm.is_online() {
            let prompt = format!(
                "New flaky test `{}` in repo {} (workflow `{}`). \
Rank these recently merged PRs by likely correlation (top 3 bullets):\n{}",
                test.test_name.as_deref().unwrap_or("(unknown)"),
                test.repo,
                test.workflow,
                pr_lines.join("\n")
            );
            llm.summarize_plain(&prompt).await.unwrap_or_default()
        } else {
            format!("Recent merges to inspect:\n{}", pr_lines.join("\n"))
        };

        let link = RegressionLink {
            id: Uuid::new_v4(),
            fingerprint: test.fingerprint.clone(),
            repo: test.repo.clone(),
            test_name: test.test_name.clone(),
            candidates_json: serde_json::to_string(&pr_lines).unwrap_or_default(),
            summary: summary.clone(),
            created_at: Utc::now(),
        };
        store.save_regression_link(&link).await?;
        links += 1;
        digest.push_report_line(&format!(
            "**{}** — {}",
            test.test_name.as_deref().unwrap_or(&test.workflow),
            summary.lines().next().unwrap_or("(see store)")
        ));
    }

    if links == 0 {
        digest.push_report_line("_No new flaky tests in 7d window._");
    }

    let final_digest = digest.finish();
    publish_digest(config, store, events, &final_digest).await?;
    Ok(RegressionLinkOutcome { links })
}
