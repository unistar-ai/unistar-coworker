//! Shared helpers for `ci_list_runs` workflow agents (main-guard, ci-efficiency).

use std::collections::HashMap;

use serde_json::json;

use crate::agent::parse::{
    format_compact_duration, parse_branch_runs, parse_compact_duration, run_conclusion_is_failure,
    ParsedBranchRun,
};
use crate::error::Result;
use crate::github::helpers::gh_tool;
use crate::github::GithubHarness;

#[derive(Debug, Clone)]
pub struct BranchRunsSample {
    pub branch: String,
    pub runs: Vec<ParsedBranchRun>,
}

#[derive(Debug, Clone, Default)]
pub struct WorkflowStats {
    pub runs: u32,
    pub failures: u32,
    pub duration_secs_sum: u64,
    pub duration_samples: u32,
    pub max_duration_secs: u64,
}

impl WorkflowStats {
    pub fn avg_duration_secs(&self) -> Option<u64> {
        if self.duration_samples == 0 {
            None
        } else {
            Some(self.duration_secs_sum / u64::from(self.duration_samples))
        }
    }
}

/// Fetch and parse recent workflow runs on a repo default branch.
pub async fn fetch_branch_runs(
    github: &GithubHarness,
    repo: &str,
    limit: u32,
) -> Result<BranchRunsSample> {
    let text = gh_tool(
        github,
        "ci_list_runs",
        json!({ "repo": repo, "limit": limit }),
    )
    .await?;
    let (branch, runs) = parse_branch_runs(&text);
    Ok(BranchRunsSample {
        branch: branch.unwrap_or_else(|| "main".into()),
        runs,
    })
}

/// Count completed runs and failures per workflow name; aggregate duration when present.
pub fn aggregate_workflow_stats(runs: &[ParsedBranchRun]) -> HashMap<String, WorkflowStats> {
    let mut by_workflow: HashMap<String, WorkflowStats> = HashMap::new();
    for run in runs {
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
        if let Some(dur) = run.duration.as_deref().and_then(parse_compact_duration) {
            entry.duration_secs_sum += dur;
            entry.duration_samples += 1;
            entry.max_duration_secs = entry.max_duration_secs.max(dur);
        }
    }
    by_workflow
}

/// Markdown table rows sorted by failures then runs (descending).
pub fn format_workflow_stats_rows(by_workflow: &HashMap<String, WorkflowStats>) -> Vec<String> {
    let mut rows: Vec<_> = by_workflow.iter().collect();
    rows.sort_by(|a, b| {
        b.1.failures
            .cmp(&a.1.failures)
            .then_with(|| b.1.runs.cmp(&a.1.runs))
    });
    rows.into_iter()
        .map(|(name, stats)| {
            let rate = if stats.runs == 0 {
                "—".into()
            } else {
                format!(
                    "{:.0}%",
                    100.0 * f64::from(stats.failures) / f64::from(stats.runs)
                )
            };
            let avg_dur = stats
                .avg_duration_secs()
                .map(format_compact_duration)
                .unwrap_or_else(|| "—".into());
            format!(
                "| {name} | {} | {} | {rate} | {avg_dur} |",
                stats.runs, stats.failures
            )
        })
        .collect()
}

/// Top workflows by average duration (requires at least one timed sample).
pub fn format_slowest_workflow_rows(
    by_workflow: &HashMap<String, WorkflowStats>,
    limit: usize,
) -> Vec<String> {
    let mut rows: Vec<_> = by_workflow
        .iter()
        .filter_map(|(name, stats)| {
            stats
                .avg_duration_secs()
                .map(|avg| (name.as_str(), avg, stats.max_duration_secs))
        })
        .collect();
    rows.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| b.2.cmp(&a.2)));
    rows.into_iter()
        .take(limit)
        .map(|(name, avg, max)| {
            format!(
                "| {name} | {} | {} |",
                format_compact_duration(avg),
                format_compact_duration(max)
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aggregate_workflow_stats_counts_failures() {
        let runs = vec![
            ParsedBranchRun {
                run_id: 1,
                workflow: "CI".into(),
                conclusion: "failure".into(),
                duration: Some("4m0s".into()),
            },
            ParsedBranchRun {
                run_id: 2,
                workflow: "CI".into(),
                conclusion: "success".into(),
                duration: Some("2m0s".into()),
            },
            ParsedBranchRun {
                run_id: 3,
                workflow: "Lint".into(),
                conclusion: "in_progress".into(),
                duration: None,
            },
        ];
        let stats = aggregate_workflow_stats(&runs);
        assert_eq!(stats["CI"].runs, 2);
        assert_eq!(stats["CI"].failures, 1);
        assert_eq!(stats["CI"].duration_samples, 2);
        assert_eq!(stats["CI"].avg_duration_secs(), Some(180));
        assert_eq!(stats["CI"].max_duration_secs, 240);
        assert!(!stats.contains_key("Lint"));
    }

    #[test]
    fn format_slowest_workflow_rows_orders_by_avg() {
        let mut by_workflow = HashMap::new();
        by_workflow.insert(
            "Fast".into(),
            WorkflowStats {
                runs: 2,
                failures: 0,
                duration_secs_sum: 120,
                duration_samples: 2,
                max_duration_secs: 70,
            },
        );
        by_workflow.insert(
            "Slow".into(),
            WorkflowStats {
                runs: 2,
                failures: 0,
                duration_secs_sum: 1200,
                duration_samples: 2,
                max_duration_secs: 700,
            },
        );
        let rows = format_slowest_workflow_rows(&by_workflow, 2);
        assert_eq!(rows.len(), 2);
        assert!(rows[0].contains("Slow"));
    }
}
