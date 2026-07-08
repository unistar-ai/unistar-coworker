use sha2::{Digest, Sha256};

use super::checks;
use super::ci_common::{self, RunSummary};
use super::ci_logs::{self, clean_gh_log, extract_errors};
use super::exec::GhExec;
use crate::error::Result;

#[derive(Debug, Clone)]
pub struct RunFailureAnalysis {
    pub run_id: u64,
    pub workflow: String,
    pub job: String,
    pub step: String,
    pub test_name: String,
    pub error_sig: String,
    pub fingerprint: String,
}

pub fn compute_failure_fingerprint(
    repo: &str,
    workflow: &str,
    job: &str,
    test_name: &str,
    error_sig: &str,
) -> String {
    let fallback = if test_name.is_empty() {
        error_sig
    } else {
        test_name
    };
    let payload = format!("{repo}|{workflow}|{job}|{fallback}");
    let hash = Sha256::digest(payload.as_bytes());
    format!("{hash:x}")
}

pub fn truncate_runes(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        s.to_string()
    } else {
        chars[..max].iter().collect()
    }
}

pub fn extract_test_name_from_logs(logs: &str) -> String {
    for line in logs.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        let low = t.to_ascii_lowercase();
        if t.contains("::") && (t.contains("FAILED") || low.contains("failed")) {
            return truncate_runes(t, 120);
        }
        if t.starts_with("FAIL ") || t.starts_with("--- FAIL:") {
            return truncate_runes(t, 120);
        }
        if t.starts_with("✕ ") || t.starts_with("× ") {
            return truncate_runes(t, 120);
        }
        if low.contains(" ... failed") && low.starts_with("test ") {
            return truncate_runes(t, 120);
        }
        if t.contains(".test.") && (low.contains(" fail") || t.starts_with("FAIL ")) {
            return truncate_runes(t, 120);
        }
        if low.contains("tests failed") || low.contains("test suite failed") {
            return truncate_runes(t, 120);
        }
        if low.starts_with("failures:") {
            return truncate_runes(t, 120);
        }
    }
    String::new()
}

pub fn extract_error_signature(logs: &str) -> String {
    let clean = clean_gh_log(logs);
    let (body, n) = extract_errors(&clean);
    let body = if n == 0 || body.trim().is_empty() {
        ci_common::tail_bytes(&clean, 500)
    } else {
        body
    };
    for line in body.lines() {
        let t = line.trim();
        if t.is_empty() || t == "…" {
            continue;
        }
        let low = t.to_ascii_lowercase();
        if low.contains("error")
            || low.contains("fail")
            || low.contains("panic")
            || low.contains("fatal")
        {
            return truncate_runes(t, 200);
        }
    }
    truncate_runes(body.trim(), 200)
}

pub async fn analyze_run_failure(
    exec: &GhExec,
    repo: &str,
    run_id: u64,
) -> Result<RunFailureAnalysis> {
    let run = ci_common::load_run_summary(exec, repo, run_id).await?;
    let (raw_logs, jobs_opt) = ci_logs::fetch_failed_run_logs(exec, repo, run_id).await?;
    let failed_jobs = jobs_opt.unwrap_or_default();

    let job = failed_jobs
        .first()
        .map(|j| j.name.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            let (_, _, _, fj) = ci_common::classify_run_jobs(&run.jobs);
            fj.first().map(|j| j.name.trim().to_string())
        })
        .unwrap_or_default();

    let step = ci_common::failed_step_names(&run.jobs)
        .first()
        .cloned()
        .unwrap_or_default();

    let test_name = extract_test_name_from_logs(&raw_logs);
    let mut error_sig = extract_error_signature(&raw_logs);
    if error_sig.is_empty() && !raw_logs.trim().is_empty() {
        error_sig = truncate_runes(raw_logs.trim(), 200);
    }

    let fp = compute_failure_fingerprint(repo, &run.workflow_name, &job, &test_name, &error_sig);
    Ok(RunFailureAnalysis {
        run_id,
        workflow: run.workflow_name,
        job,
        step,
        test_name,
        error_sig,
        fingerprint: fp,
    })
}

pub fn format_failure_analysis(a: &RunFailureAnalysis) -> String {
    let mut out = format!("Run {}  {}\n", a.run_id, a.workflow);
    if !a.job.is_empty() {
        out.push_str(&format!("Job: {}\n", a.job));
    }
    if !a.step.is_empty() {
        out.push_str(&format!("Step: {}\n", a.step));
    }
    if !a.test_name.is_empty() {
        out.push_str(&format!("Test: {}\n", a.test_name));
    }
    if !a.error_sig.is_empty() {
        out.push_str(&format!("Error signature: {}\n", a.error_sig));
    }
    out.push_str(&format!("Fingerprint: {}\n", a.fingerprint));
    out.push_str("Next: policy_classify_failure; then ci_compare_runs or ci_get_failed_logs.");
    out.trim().to_string()
}

pub fn format_failure_log_synopsis(
    repo: &str,
    run: &RunSummary,
    run_id: u64,
    target_jobs: &[ci_common::RunJob],
    distilled: &str,
) -> String {
    let mut target_jobs: Vec<ci_common::RunJob> = target_jobs.to_vec();
    if target_jobs.is_empty() {
        let (_, _, _, fj) = ci_common::classify_run_jobs(&run.jobs);
        target_jobs = fj;
    }

    let mut out = format!(
        "Run {run_id}  {}  branch:{}\n",
        run.workflow_name, run.head_branch
    );
    for j in &target_jobs {
        out.push_str(&format!(
            "Failed job: {} (job_id={})\n",
            j.name, j.database_id
        ));
    }
    let steps = ci_common::failed_step_names(&target_jobs);
    if !steps.is_empty() {
        out.push_str(&format!("Failed steps: {}\n", steps.join(", ")));
    }

    let test_name = extract_test_name_from_logs(distilled);
    if !test_name.is_empty() {
        out.push_str(&format!("Test: {test_name}\n"));
    }
    let sig = extract_error_signature(distilled);
    if !sig.is_empty() {
        out.push_str(&format!("Sig: {sig}\n"));
    }

    let job_name = target_jobs.first().map(|j| j.name.as_str()).unwrap_or("");
    let fp = compute_failure_fingerprint(repo, &run.workflow_name, job_name, &test_name, &sig);
    out.push_str(&format!("FP: {fp}\n"));
    out.push_str("Next: policy_classify_failure; ci_get_job_logs for deeper single-job logs\n---");
    out.trim().to_string()
}

use serde_json::Value;

use super::args::{require_str, require_u32, require_u64};
use super::error::{format_tool_error, ErrCode};

pub async fn ci_failure_fingerprint(exec: &GhExec, args: &Value) -> Result<String> {
    let repo = require_str(args, "repo")?;
    let run_id = require_u64(args, "run_id")?;
    let analysis = analyze_run_failure(exec, &repo, run_id).await?;
    Ok(format_failure_analysis(&analysis))
}

pub async fn ci_compare_runs(exec: &GhExec, args: &Value) -> Result<String> {
    let repo = require_str(args, "repo")?;
    let run_a = require_u64(args, "run_id_a")?;
    let run_b = require_u64(args, "run_id_b")?;

    let analysis_a = analyze_run_failure(exec, &repo, run_a).await.map_err(|e| {
        crate::error::CoworkerError::Other(anyhow::anyhow!(format_tool_error(
            ErrCode::NotFound,
            &format!("run_id_a {run_a}: {e}"),
            "Confirm run IDs from ci_analyze_pr_failures or ci_list_runs",
        )))
    })?;
    let analysis_b = analyze_run_failure(exec, &repo, run_b).await.map_err(|e| {
        crate::error::CoworkerError::Other(anyhow::anyhow!(format_tool_error(
            ErrCode::NotFound,
            &format!("run_id_b {run_b}: {e}"),
            "Confirm run IDs from ci_analyze_pr_failures or ci_list_runs",
        )))
    })?;

    let same_fp =
        analysis_a.fingerprint == analysis_b.fingerprint && !analysis_a.fingerprint.is_empty();
    let same_workflow = analysis_a.workflow == analysis_b.workflow;

    let mut out = format!("Compare runs in {repo}\n\n");
    out.push_str(&format!(
        "Run A: {}  {}\n",
        analysis_a.run_id, analysis_a.workflow
    ));
    out.push_str(&format!("  Fingerprint: {}\n", analysis_a.fingerprint));
    if !analysis_a.job.is_empty() {
        out.push_str(&format!("  Job: {}\n", analysis_a.job));
    }
    out.push_str(&format!(
        "Run B: {}  {}\n",
        analysis_b.run_id, analysis_b.workflow
    ));
    out.push_str(&format!("  Fingerprint: {}\n", analysis_b.fingerprint));
    if !analysis_b.job.is_empty() {
        out.push_str(&format!("  Job: {}\n", analysis_b.job));
    }

    out.push('\n');
    if same_fp {
        out.push_str("Same fingerprint: yes — likely the same failure (possibly flaky).\n");
    } else {
        out.push_str("Same fingerprint: no — failures differ.\n");
    }
    if same_workflow {
        out.push_str("Same workflow: yes\n");
    } else {
        out.push_str(&format!(
            "Same workflow: no ({} vs {})\n",
            analysis_a.workflow, analysis_b.workflow
        ));
    }

    if !analysis_a.job.is_empty() && !analysis_b.job.is_empty() {
        if analysis_a.job == analysis_b.job {
            out.push_str(&format!("Failed job: both {}\n", analysis_a.job));
        } else {
            out.push_str(&format!(
                "Failed job: A={}  B={}\n",
                analysis_a.job, analysis_b.job
            ));
        }
    }

    if !analysis_a.error_sig.is_empty()
        && !analysis_b.error_sig.is_empty()
        && analysis_a.error_sig != analysis_b.error_sig
    {
        out.push_str("\nError signature changed — inspect ci_get_failed_logs if unsure.\n");
    } else if same_fp {
        out.push_str("\nNext: ci_rerun_workflow if flaky; otherwise fix the recurring failure.\n");
    }

    Ok(out.trim().to_string())
}

pub async fn ci_list_external_checks(exec: &GhExec, args: &Value) -> Result<String> {
    let repo = require_str(args, "repo")?;
    let pr_num = require_u32(args, "pr_number")?;
    let rollup = ci_common::pr_status_rollup(exec, &repo, pr_num).await?;

    let lines: Vec<String> = rollup
        .iter()
        .filter(|c| c.typename.as_deref() == Some("StatusContext"))
        .filter_map(|c| {
            let name = checks::check_display_name(c);
            if name.is_empty() {
                None
            } else {
                Some(format!(
                    "- {}: {}",
                    name,
                    checks::check_verdict(c).to_ascii_lowercase()
                ))
            }
        })
        .collect();

    if lines.is_empty() {
        return Ok(format!(
            "No external status checks on PR #{pr_num} in {repo}.\nGitHub Actions checks are not listed here — use ci_analyze_pr_failures."
        ));
    }

    let mut out = format!(
        "{} external check(s) on PR #{pr_num} in {repo}:\n",
        lines.len()
    );
    out.push_str(&lines.join("\n"));
    out.push_str("\n\nThese are not GitHub Actions — do not call ci_get_failed_logs.");
    out.push_str("\nInspect the PR checks tab or the external CI system for logs and rerun.");
    Ok(out.trim().to_string())
}
