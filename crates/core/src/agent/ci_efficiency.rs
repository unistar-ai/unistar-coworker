use crate::agent::ci_runs::{
    aggregate_workflow_stats, fetch_branch_runs, format_slowest_workflow_rows,
    format_workflow_stats_rows,
};
use crate::config::Config;
use crate::error::{CoworkerError, Result};
use crate::github::GithubHarness;

/// Build CI efficiency markdown without publishing a digest (for `report ci`).
pub async fn build_ci_efficiency_markdown(
    config: &Config,
    github: &GithubHarness,
) -> Result<String> {
    if !github.is_available() {
        return Err(CoworkerError::Workflow(
            "GitHub harness (gh) is required for CI report".into(),
        ));
    }

    let limit = config.main_guard.recent_runs.clamp(10, 50);
    let mut out = String::from("# CI efficiency report\n\n");

    for repo in config.repos.clone() {
        let sample = fetch_branch_runs(github, &repo, limit).await?;
        out.push_str(&format!("## {repo} ({})\n\n", sample.branch));

        let by_workflow = aggregate_workflow_stats(&sample.runs);
        if by_workflow.is_empty() {
            out.push_str("_No completed runs in sample._\n\n");
            continue;
        }

        out.push_str("| Workflow | Runs | Failures | Fail rate | Avg duration |\n");
        out.push_str("|----------|------|----------|-----------|--------------|\n");
        for row in format_workflow_stats_rows(&by_workflow) {
            out.push_str(&row);
            out.push('\n');
        }

        let slowest = format_slowest_workflow_rows(&by_workflow, 5);
        if !slowest.is_empty() {
            out.push_str("\n**Slowest workflows (avg duration)**\n\n");
            out.push_str("| Workflow | Avg | Max |\n");
            out.push_str("|----------|-----|-----|\n");
            for row in slowest {
                out.push_str(&row);
                out.push('\n');
            }
        }
        out.push('\n');
    }

    Ok(out)
}
