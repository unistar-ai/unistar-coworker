use std::collections::HashMap;

use tokio::sync::broadcast;

use crate::agent::parse::{parse_branch_runs, run_conclusion_is_failure};
use crate::app::AppEvent;
use crate::config::Config;
use crate::engine::AgentSpec;
use crate::error::{CoworkerError, Result};
use crate::mcp::helpers::lazy_tool;
use crate::mcp::McpClient;
use crate::output::digest::{publish_digest, IncrementalDigest};
use crate::store::Store;

#[derive(Debug, Default)]
struct WorkflowStats {
    runs: u32,
    failures: u32,
}

pub struct CiEfficiencyOutcome {
    pub repos: u32,
    pub workflows: u32,
}

impl CiEfficiencyOutcome {
    pub fn format_summary(&self) -> String {
        format!(
            "ci-efficiency: {} repo(s), {} workflow(s) in report",
            self.repos, self.workflows
        )
    }
}

pub async fn run_ci_efficiency(
    config: &Config,
    mcp: &dyn McpClient,
    store: &dyn Store,
    agent: &AgentSpec,
    events: &broadcast::Sender<AppEvent>,
    log: impl Fn(&str, String),
) -> Result<CiEfficiencyOutcome> {
    if !mcp.is_available() {
        return Err(CoworkerError::Workflow(
            "unistar-mcp is required for ci-efficiency".into(),
        ));
    }

    let limit = config.main_guard.recent_runs.clamp(10, 50);
    let mut digest = IncrementalDigest::begin(agent);
    let mut repo_count = 0u32;
    let mut workflow_count = 0u32;

    for repo in config.repos.clone() {
        log("info", format!("ci-efficiency: sampling runs for {repo}"));
        let text = lazy_tool(
            mcp,
            "ci_list_runs",
            serde_json::json!({ "repo": repo, "limit": limit }),
        )
        .await?;

        let (branch, runs) = parse_branch_runs(&text);
        let branch = branch.unwrap_or_else(|| "main".into());
        digest.begin_repo(&repo);

        let mut by_workflow: HashMap<String, WorkflowStats> = HashMap::new();
        for run in &runs {
            let c = run.conclusion.to_ascii_lowercase();
            if c.is_empty() || matches!(c.as_str(), "in_progress" | "queued" | "waiting" | "pending") {
                continue;
            }
            let entry = by_workflow.entry(run.workflow.clone()).or_default();
            entry.runs += 1;
            if run_conclusion_is_failure(&c) {
                entry.failures += 1;
            }
        }

        if by_workflow.is_empty() {
            digest.push_report_line("_No completed runs in sample._");
            continue;
        }

        repo_count += 1;
        digest.push_report_line(&format!("Branch: **{branch}** (last {limit} runs sample)"));
        digest.push_report_line("| Workflow | Runs | Failures | Fail rate |");
        digest.push_report_line("|----------|------|----------|-----------|");

        let mut rows: Vec<_> = by_workflow.iter().collect();
        rows.sort_by(|a, b| {
            b.1.failures
                .cmp(&a.1.failures)
                .then_with(|| b.1.runs.cmp(&a.1.runs))
        });

        for (name, stats) in rows {
            workflow_count += 1;
            let rate = if stats.runs == 0 {
                "—".into()
            } else {
                format!("{:.0}%", 100.0 * f64::from(stats.failures) / f64::from(stats.runs))
            };
            digest.push_report_line(&format!(
                "| {name} | {} | {} | {rate} |",
                stats.runs, stats.failures
            ));
        }
    }

    let final_digest = digest.finish();
    publish_digest(config, store, events, &final_digest).await?;

    Ok(CiEfficiencyOutcome {
        repos: repo_count,
        workflows: workflow_count,
    })
}

/// Build CI efficiency markdown without publishing a digest (for `report ci`).
pub async fn build_ci_efficiency_markdown(
    config: &Config,
    mcp: &dyn McpClient,
) -> Result<String> {
    if !mcp.is_available() {
        return Err(CoworkerError::Workflow(
            "unistar-mcp is required for CI report".into(),
        ));
    }

    let limit = config.main_guard.recent_runs.clamp(10, 50);
    let mut out = String::from("# CI efficiency report\n\n");

    for repo in config.repos.clone() {
        let text = lazy_tool(
            mcp,
            "ci_list_runs",
            serde_json::json!({ "repo": repo, "limit": limit }),
        )
        .await?;

        let (branch, runs) = parse_branch_runs(&text);
        let branch = branch.unwrap_or_else(|| "main".into());
        out.push_str(&format!("## {repo} ({branch})\n\n"));

        let mut by_workflow: HashMap<String, WorkflowStats> = HashMap::new();
        for run in &runs {
            let c = run.conclusion.to_ascii_lowercase();
            if c.is_empty()
                || matches!(
                    c.as_str(),
                    "in_progress" | "queued" | "waiting" | "pending"
                )
            {
                continue;
            }
            let entry = by_workflow.entry(run.workflow.clone()).or_default();
            entry.runs += 1;
            if run_conclusion_is_failure(&c) {
                entry.failures += 1;
            }
        }

        if by_workflow.is_empty() {
            out.push_str("_No completed runs in sample._\n\n");
            continue;
        }

        out.push_str("| Workflow | Runs | Failures | Fail rate |\n");
        out.push_str("|----------|------|----------|-----------|\n");

        let mut rows: Vec<_> = by_workflow.iter().collect();
        rows.sort_by(|a, b| {
            b.1.failures
                .cmp(&a.1.failures)
                .then_with(|| b.1.runs.cmp(&a.1.runs))
        });

        for (name, stats) in rows {
            let rate = if stats.runs == 0 {
                "—".into()
            } else {
                format!("{:.0}%", 100.0 * f64::from(stats.failures) / f64::from(stats.runs))
            };
            out.push_str(&format!(
                "| {name} | {} | {} | {rate} |\n",
                stats.runs, stats.failures
            ));
        }
        out.push('\n');
    }

    Ok(out)
}
