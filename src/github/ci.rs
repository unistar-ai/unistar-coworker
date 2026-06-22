use serde_json::Value;

use super::args::{
    optional_bool, optional_i64, optional_str, optional_u32, require_str, require_u32, require_u64,
};
use super::checks::{
    self, external_checks_failing, format_external_check_summary, pending_check_summary,
};
use super::ci_common::{
    self, CI_RUN_LIST_LIMIT, WorkflowRun,
};
use super::ci_logs::{self, DistillOptions};
use super::ci_fingerprint;
use super::error::format_tool_ok;
use super::exec::GhExec;
use crate::error::Result;

pub use ci_common::{default_branch, failing_runs_for_pr, list_branch_runs, load_run_summary};

#[derive(Debug)]
pub struct PrFailureState {
    pub pr_num: u32,
    pub head_sha: String,
    pub real_failed: Vec<WorkflowRun>,
    pub waiting_approval: Vec<WorkflowRun>,
    pub rollup: Vec<checks::CheckRollup>,
    pub truncated: bool,
}

pub async fn load_pr_failure_state(
    exec: &GhExec,
    repo: &str,
    pr_num: u32,
    include_external: bool,
) -> Result<PrFailureState> {
    let (head_sha, failed, truncated) = failing_runs_for_pr(exec, repo, pr_num).await?;
    let mut state = PrFailureState {
        pr_num,
        head_sha,
        real_failed: Vec::new(),
        waiting_approval: Vec::new(),
        rollup: Vec::new(),
        truncated,
    };
    for r in failed {
        let conc = r.conclusion.trim().to_ascii_lowercase();
        if conc == "action_required" {
            state.waiting_approval.push(r);
        } else {
            state.real_failed.push(r);
        }
    }
    if include_external {
        state.rollup = ci_common::pr_status_rollup(exec, repo, pr_num).await.unwrap_or_default();
    }
    state
        .real_failed
        .sort_by(|a, b| a.workflow_name.cmp(&b.workflow_name));
    state
        .waiting_approval
        .sort_by(|a, b| a.workflow_name.cmp(&b.workflow_name));
    Ok(state)
}

pub fn compute_ci_kind(real_failed: usize, waiting_approval: usize, rollup: &[checks::CheckRollup]) -> String {
    let has_actions = real_failed > 0;
    let has_external = external_checks_failing(rollup);
    let has_approval = waiting_approval > 0;
    let has_pending = !pending_check_summary(rollup).is_empty();

    if has_actions && has_external {
        "mixed".into()
    } else if has_actions {
        "actions_only".into()
    } else if has_external {
        "external_only".into()
    } else if has_approval {
        "approval".into()
    } else if has_pending {
        "pending".into()
    } else {
        "clean".into()
    }
}

fn prepend_ci_kind(body: &str, kind: &str) -> String {
    let body = body.trim();
    if body.is_empty() {
        format!("CI_KIND: {kind}")
    } else {
        format!("CI_KIND: {kind}\n{body}")
    }
}

pub fn format_analyze_pr_failures(state: &PrFailureState, include_external: bool) -> String {
    if state.real_failed.is_empty() && state.waiting_approval.is_empty() {
        let mut out = format!(
            "No failing GitHub Actions runs for PR #{} @{}.\n",
            state.pr_num,
            checks::short_sha(&state.head_sha)
        );
        let ext = format_external_check_summary(&state.rollup);
        if !ext.is_empty() {
            out.push('\n');
            out.push_str(&ext);
            out.push_str("Do not call ci_get_failed_logs for external checks — inspect the PR checks tab.\n");
        } else {
            let pending = pending_check_summary(&state.rollup);
            if !pending.is_empty() {
                out.push('\n');
                out.push_str(&pending);
            } else {
                out.push_str("If pr_get_status reports failing checks, they may come from an external CI system; inspect the PR page.\n");
            }
        }
        let kind = compute_ci_kind(0, 0, &state.rollup);
        return prepend_ci_kind(out.trim(), &kind);
    }

    let mut out = String::new();
    if !state.real_failed.is_empty() {
        out.push_str(&format!(
            "{} failing run(s) for PR #{} @{}:\n",
            state.real_failed.len(),
            state.pr_num,
            checks::short_sha(&state.head_sha)
        ));
        if state.truncated {
            out.push_str(&format!(
                "(only the most recent {CI_RUN_LIST_LIMIT} runs were inspected; there may be more)\n"
            ));
        }
        for r in &state.real_failed {
            let mut label = r.conclusion.trim().to_ascii_lowercase();
            if label.is_empty() {
                label = r.status.trim().to_ascii_lowercase();
            }
            out.push_str(&format!(
                "{}  {}  {label}\n",
                r.database_id, r.workflow_name
            ));
        }
    }
    if !state.waiting_approval.is_empty() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&format!(
            "{} run(s) waiting for approval (action_required — not a code failure; do not call ci_get_failed_logs):\n",
            state.waiting_approval.len()
        ));
        for r in &state.waiting_approval {
            out.push_str(&format!("{}  {}  action_required\n", r.database_id, r.workflow_name));
        }
    }
    if include_external {
        let ext = format_external_check_summary(&state.rollup);
        if !ext.is_empty() {
            out.push('\n');
            out.push_str(&ext);
        }
    }
    let kind = compute_ci_kind(
        state.real_failed.len(),
        state.waiting_approval.len(),
        &state.rollup,
    );
    prepend_ci_kind(out.trim(), &kind)
}

pub async fn ci_analyze_pr_failures(exec: &GhExec, args: &Value) -> Result<String> {
    let repo = require_str(args, "repo")?;
    let pr_num = require_u32(args, "pr_number")?;
    let include_external = optional_bool(args, "include_external", true);
    let state = load_pr_failure_state(exec, &repo, pr_num, include_external).await?;
    Ok(format_analyze_pr_failures(&state, include_external))
}

pub async fn ci_get_run_summary(exec: &GhExec, args: &Value) -> Result<String> {
    let repo = require_str(args, "repo")?;
    let run_id = require_u64(args, "run_id")?;
    let run = load_run_summary(exec, &repo, run_id).await?;

    let mut conclusion = run.conclusion.trim().to_ascii_lowercase();
    if conclusion.is_empty() {
        conclusion = run.status.trim().to_ascii_lowercase();
    }

    let mut out = format!("Run {}  {}\n", run.database_id, run.workflow_name);
    out.push_str(&format!("Branch: {}\n", run.head_branch));
    out.push_str(&format!(
        "Status: {}  Conclusion: {conclusion}\n",
        run.status.to_ascii_lowercase()
    ));
    if !run.created_at.is_empty() && !run.updated_at.is_empty() {
        out.push_str(&format!(
            "Started: {}  Updated: {}\n",
            run.created_at, run.updated_at
        ));
    }

    let (success, failed, pending, failed_jobs) = ci_common::classify_run_jobs(&run.jobs);
    out.push_str(&format!(
        "Jobs: {success} success / {failed} failed / {pending} pending\n"
    ));
    if !failed_jobs.is_empty() {
        out.push_str("Failed jobs:\n");
        for j in &failed_jobs {
            out.push_str(&format!("- {}  (job_id={})\n", j.name, j.database_id));
        }
        let steps = ci_common::failed_step_names(&run.jobs);
        if !steps.is_empty() {
            out.push_str("Failed steps:\n");
            for s in steps {
                out.push_str(&format!("  - {s}\n"));
            }
        }
        if ci_common::run_still_in_progress(&run) && failed > 0 {
            out.push_str(
                "Note: run is still in progress; ci_get_failed_logs can fetch logs for failed jobs that have finished.\n",
            );
        }
    }
    Ok(out.trim().to_string())
}

pub async fn ci_get_failed_logs(exec: &GhExec, args: &Value) -> Result<String> {
    let repo = require_str(args, "repo")?;
    let run_id = require_u64(args, "run_id")?;
    let job_id = optional_i64(args, "job_id", 0) as u64;
    let focus = optional_str(args, "focus").unwrap_or_else(|| "last".into());
    let offset_lines = args
        .get("offset_lines")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;
    let max_lines = args
        .get("max_lines")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;

    let run = load_run_summary(exec, &repo, run_id).await?;
    let (log_text, failed_jobs) = ci_logs::fetch_failed_logs(exec, &repo, run_id, job_id).await?;

    if log_text.trim().is_empty() {
        if !failed_jobs.is_empty() {
            let synopsis =
                ci_fingerprint::format_failure_log_synopsis(&repo, &run, run_id, &failed_jobs, "");
            let names: Vec<String> = failed_jobs
                .iter()
                .map(|j| {
                    let mut state = ci_common::job_effective_conclusion(j);
                    if !ci_common::job_logs_ready(j) {
                        state.push_str(", logs pending");
                    }
                    format!("{} (job_id={}, {state})", j.name, j.database_id)
                })
                .collect();
            return Ok(format!(
                "{synopsis}\n\nNo log output yet for {} failed job(s): {}",
                failed_jobs.len(),
                names.join("; ")
            ));
        }
        let synopsis =
            ci_fingerprint::format_failure_log_synopsis(&repo, &run, run_id, &[], "");
        return Ok(format!(
            "{synopsis}\n\nRun {run_id} has no failed-step logs (still running or cancelled)."
        ));
    }

    let merged_jobs = ci_logs::merge_jobs_for_distill(&run.jobs, &failed_jobs);
    let opts = DistillOptions {
        focus: &focus,
        jobs: &merged_jobs,
    };
    let (body, mode) = ci_logs::distill_failed_log_text(&log_text, opts);
    let synopsis =
        ci_fingerprint::format_failure_log_synopsis(&repo, &run, run_id, &failed_jobs, &body);
    Ok(ci_logs::format_failed_logs_response(
        run_id,
        &synopsis,
        &body,
        mode,
        offset_lines,
        max_lines,
    ))
}

pub async fn ci_list_runs(exec: &GhExec, args: &Value) -> Result<String> {
    let repo = require_str(args, "repo")?;
    let mut branch = optional_str(args, "branch").unwrap_or_default();
    if branch.is_empty() {
        branch = default_branch(exec, &repo).await?;
    }
    let limit = optional_u32(args, "limit", 15).min(50);
    let runs = list_branch_runs(exec, &repo, &branch, limit).await?;

    let mut out = format!(
        "branch: {branch}\n{} run(s) for {repo}:\n",
        runs.len()
    );
    for r in &runs {
        let conclusion = ci_common::run_conclusion(r);
        let dur = ci_common::format_run_duration(&r.created_at, &r.updated_at, &conclusion);
        let branch_note = if !r.head_branch.is_empty() && r.head_branch != branch {
            format!("  branch:{}", r.head_branch)
        } else {
            String::new()
        };
        out.push_str(&format!(
            "{}  {}  {conclusion}  {dur}{branch_note}\n",
            r.database_id, r.workflow_name
        ));
    }
    Ok(out.trim().to_string())
}

pub async fn ci_rerun_workflow(exec: &GhExec, args: &Value) -> Result<String> {
    let repo = require_str(args, "repo")?;
    let run_id = require_u64(args, "run_id")?;
    let run_s = run_id.to_string();
    let res = exec
        .run_retry(&["run", "rerun", &run_s, "-R", &repo, "--failed"])
        .await;
    GhExec::into_result(res, "failed to rerun workflow")?;
    Ok(format_tool_ok(&format!("Reran failed jobs in run {run_id}.")))
}
