use serde_json::Value;

use super::args::{optional_bool, optional_u32, require_str, require_u32, require_u64};
use super::ci;
use super::ci_common;
use super::ci_fingerprint::{self, RunFailureAnalysis};
use super::ci_logs::{self, DistillOptions};
use super::exec::GhExec;
use super::policy;
use crate::error::Result;

const FAILURE_DIGEST_EXCERPT_BUDGET: usize = 1_024;
const DEFAULT_CI_SNAPSHOT_MAX_RUNS: u32 = 2;
const MAX_CI_SNAPSHOT_RUNS: u32 = 5;

pub fn format_flaky_fingerprint_hint(_repo: &str, fingerprint: &str) -> String {
    if fingerprint.is_empty() {
        return String::new();
    }
    "Flaky hint: new fingerprint in webhook ledger (or webhook not configured)".into()
}

pub async fn build_failure_digest_text(
    exec: &GhExec,
    repo: &str,
    run_id: u64,
    job_id: u64,
) -> Result<String> {
    let run = ci_common::load_run_summary(exec, repo, run_id).await?;
    let (log_text, failed_jobs) = ci_logs::fetch_failed_logs(exec, repo, run_id, job_id).await?;

    let merged_jobs = ci_logs::merge_jobs_for_distill(&run.jobs, &failed_jobs);
    let opts = DistillOptions {
        focus: "last",
        jobs: &merged_jobs,
    };
    let (body, _) = ci_logs::distill_failed_log_text(&log_text, opts);
    let synopsis =
        ci_fingerprint::format_failure_log_synopsis(repo, &run, run_id, &failed_jobs, &body);

    let mut analysis = RunFailureAnalysis {
        run_id,
        workflow: run.workflow_name.clone(),
        job: String::new(),
        step: String::new(),
        test_name: ci_fingerprint::extract_test_name_from_logs(&body),
        error_sig: ci_fingerprint::extract_error_signature(&body),
        fingerprint: String::new(),
    };
    if let Some(j) = failed_jobs.first() {
        analysis.job = j.name.clone();
    }
    let steps = ci_common::failed_step_names(&failed_jobs);
    if let Some(s) = steps.first() {
        analysis.step = s.clone();
    }
    if analysis.error_sig.is_empty() && !body.trim().is_empty() {
        analysis.error_sig = ci_fingerprint::truncate_runes(body.trim(), 200);
    }
    analysis.fingerprint = ci_fingerprint::compute_failure_fingerprint(
        repo,
        &analysis.workflow,
        &analysis.job,
        &analysis.test_name,
        &analysis.error_sig,
    );

    let (verdict, rule_id) = policy::classify_failure(&analysis);

    let mut out = synopsis;
    let hint = format_flaky_fingerprint_hint(repo, &analysis.fingerprint);
    if !hint.is_empty() {
        out.push('\n');
        out.push_str(&hint);
    }
    out.push_str(&format!("\nVerdict: {} ({rule_id})\n", verdict.as_str()));
    let excerpt = body.trim();
    if !excerpt.is_empty() {
        out.push_str("\nExcerpt:\n");
        out.push_str(&ci_common::tail_bytes(
            excerpt,
            FAILURE_DIGEST_EXCERPT_BUDGET,
        ));
    } else if !failed_jobs.is_empty() {
        out.push_str("\n(no log excerpt yet — job may still be running)\n");
    }
    Ok(out.trim().to_string())
}

pub async fn ci_get_failure_digest(exec: &GhExec, args: &Value) -> Result<String> {
    let repo = require_str(args, "repo")?;
    let run_id = require_u64(args, "run_id")?;
    let job_id = args.get("job_id").and_then(|v| v.as_u64()).unwrap_or(0);

    let mut text = build_failure_digest_text(exec, &repo, run_id, job_id).await?;
    text.push_str("\nNext: ci_get_failed_logs for full excerpts; ci_rerun_workflow if flaky.");
    Ok(text.trim().to_string())
}

pub async fn build_pr_ci_snapshot_text(
    exec: &GhExec,
    repo: &str,
    pr_num: u32,
    max_runs: u32,
    include_external: bool,
) -> Result<String> {
    let mut max_runs = max_runs;
    if max_runs == 0 {
        max_runs = DEFAULT_CI_SNAPSHOT_MAX_RUNS;
    }
    if max_runs > MAX_CI_SNAPSHOT_RUNS {
        max_runs = MAX_CI_SNAPSHOT_RUNS;
    }

    let state = ci::load_pr_failure_state(exec, repo, pr_num, include_external).await?;
    let mut out = ci::format_analyze_pr_failures(&state, include_external);
    if state.real_failed.is_empty() {
        return Ok(out.trim().to_string());
    }

    let n = state.real_failed.len().min(max_runs as usize);
    for i in 0..n {
        let r = &state.real_failed[i];
        out.push_str(&format!(
            "\n\n--- run {} ({}) ---\n",
            r.database_id, r.workflow_name
        ));
        match build_failure_digest_text(exec, repo, r.database_id, 0).await {
            Ok(digest) => out.push_str(&digest),
            Err(e) => out.push_str(&format!("(digest unavailable: {e})\n")),
        }
    }
    if state.real_failed.len() > n {
        out.push_str(&format!(
            "\n\n({} more failing run(s) — call ci_get_failure_digest per run_id)\n",
            state.real_failed.len() - n
        ));
    }
    out.push_str("\nNext: ci_get_failed_logs for full excerpts; ci_rerun_workflow if flaky.");
    Ok(out.trim().to_string())
}

pub async fn pr_get_ci_snapshot(exec: &GhExec, args: &Value) -> Result<String> {
    let repo = require_str(args, "repo")?;
    let pr_num = require_u32(args, "pr_number")?;
    let max_runs = optional_u32(args, "max_runs", DEFAULT_CI_SNAPSHOT_MAX_RUNS);
    let include_external = optional_bool(args, "include_external", true);
    build_pr_ci_snapshot_text(exec, &repo, pr_num, max_runs, include_external).await
}
