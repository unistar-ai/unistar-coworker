use serde_json::Value;

use super::args::{optional_str, optional_u32, require_str};
use super::ci_common::{self, BranchRun};
use super::exec::GhExec;
use crate::error::Result;

pub async fn ci_branch_health(exec: &GhExec, args: &Value) -> Result<String> {
    let repo = require_str(args, "repo")?;
    let mut branch = optional_str(args, "branch").unwrap_or_default();
    if branch.is_empty() {
        branch = ci_common::default_branch(exec, &repo).await?;
    }
    let limit = optional_u32(args, "limit", 15);
    let runs = ci_common::list_branch_runs(exec, &repo, &branch, limit).await?;
    Ok(build_branch_health_text(&repo, &branch, &runs))
}

fn build_branch_health_text(repo: &str, branch: &str, runs: &[BranchRun]) -> String {
    let mut lines = vec![format!("Branch health: {repo}  {branch}")];
    if runs.is_empty() {
        lines.push("No workflow runs found.".into());
        lines.push("hint: confirm branch name or widen limit with ci_list_runs".into());
        return lines.join("\n");
    }

    let mut completed = 0u32;
    let mut failed = 0u32;
    let mut streak = 0u32;
    let mut streak_done = false;
    let mut last_fail_id = 0u64;
    let mut last_fail_wf = String::new();
    let mut slowest_name = String::new();
    let mut slowest_dur_str = String::new();
    let mut slowest_dur = std::time::Duration::ZERO;

    for r in runs {
        let c = ci_common::run_conclusion(r);
        if ci_common::run_status_in_progress(&c) {
            continue;
        }
        completed += 1;
        if ci_common::is_failed_conclusion(&c) {
            failed += 1;
            if !streak_done {
                streak += 1;
            }
            if last_fail_id == 0 {
                last_fail_id = r.database_id;
                last_fail_wf = r.workflow_name.clone();
            }
        } else if !streak_done {
            streak_done = true;
        }
        let dur = ci_common::run_duration(&r.created_at, &r.updated_at, &c);
        if dur > slowest_dur {
            slowest_dur = dur;
            slowest_name = r.workflow_name.clone();
            slowest_dur_str =
                ci_common::format_run_duration(&r.created_at, &r.updated_at, &c);
        }
    }

    let fail_rate = if completed > 0 {
        format!(
            "{failed}/{completed} ({:.0}%)",
            failed as f64 * 100.0 / completed as f64
        )
    } else {
        "-".into()
    };
    lines.push(format!(
        "Recent runs: {} listed, {completed} completed, failures: {fail_rate}",
        runs.len()
    ));
    if streak > 0 {
        lines.push(format!("Failure streak (newest first): {streak}"));
    }
    if last_fail_id != 0 {
        lines.push(format!(
            "Last failure: run_id={last_fail_id} workflow={last_fail_wf}"
        ));
    }
    if !slowest_name.is_empty() {
        lines.push(format!(
            "Slowest recent run: {slowest_name} ({slowest_dur_str})"
        ));
    }
    lines.push("Next: ci_get_run_summary on last failure; ci_list_runs for raw lines.".into());
    lines.join("\n")
}
