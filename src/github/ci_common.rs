use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::Deserialize;

use super::checks::{self, CheckRollup};
use super::exec::{GhExec, RunResult};
use crate::error::{CoworkerError, Result};

#[derive(Debug, Clone, Deserialize)]
pub struct BranchRun {
    #[serde(rename = "databaseId")]
    pub database_id: u64,
    #[serde(rename = "workflowName")]
    pub workflow_name: String,
    #[serde(default)]
    pub conclusion: String,
    #[serde(default)]
    pub status: String,
    #[serde(default, rename = "headBranch")]
    pub head_branch: String,
    #[serde(default, rename = "createdAt")]
    pub created_at: String,
    #[serde(default, rename = "updatedAt")]
    pub updated_at: String,
}

pub const CI_RUN_LIST_LIMIT: u32 = 100;

#[derive(Debug, Clone, Deserialize)]
pub struct WorkflowRun {
    #[serde(rename = "databaseId")]
    pub database_id: u64,
    #[serde(rename = "workflowName")]
    pub workflow_name: String,
    #[serde(default)]
    pub conclusion: String,
    #[serde(default)]
    pub status: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RunStep {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub conclusion: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RunJob {
    #[serde(rename = "databaseId")]
    pub database_id: u64,
    pub name: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub conclusion: String,
    #[serde(default)]
    pub steps: Vec<RunStep>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RunSummary {
    #[serde(rename = "databaseId")]
    pub database_id: u64,
    #[serde(rename = "workflowName")]
    pub workflow_name: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub conclusion: String,
    #[serde(default, rename = "createdAt")]
    pub created_at: String,
    #[serde(default, rename = "updatedAt")]
    pub updated_at: String,
    #[serde(default, rename = "headBranch")]
    pub head_branch: String,
    #[serde(default)]
    pub jobs: Vec<RunJob>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PrAuthor {
    pub login: String,
}

pub async fn pr_head_sha(exec: &GhExec, repo: &str, pr_num: u32) -> Result<String> {
    let pr_s = pr_num.to_string();
    let args = [
        "pr",
        "view",
        &pr_s,
        "-R",
        repo,
        "--json",
        "headRefOid",
        "-q",
        ".headRefOid",
    ];
    let res = exec.run_retry(&args).await;
    let stdout = GhExec::into_result(res, "failed to resolve PR head commit")?;
    Ok(stdout.trim().to_string())
}

pub async fn failing_runs_for_pr(
    exec: &GhExec,
    repo: &str,
    pr_num: u32,
) -> Result<(String, Vec<WorkflowRun>, bool)> {
    let head_sha = pr_head_sha(exec, repo, pr_num).await?;
    let limit_s = CI_RUN_LIST_LIMIT.to_string();
    let args = [
        "run",
        "list",
        "-R",
        repo,
        "--commit",
        &head_sha,
        "--limit",
        &limit_s,
        "--json",
        "databaseId,workflowName,conclusion,status",
    ];
    let res = exec.run_retry(&args).await;
    let stdout = GhExec::into_result(res, "failed to list workflow runs")?;
    let runs: Vec<WorkflowRun> = serde_json::from_str(&stdout)
        .map_err(|e| CoworkerError::Other(anyhow::anyhow!("failed to parse run list: {e}")))?;
    let truncated = runs.len() == CI_RUN_LIST_LIMIT as usize;

    let mut failed = Vec::new();
    for mut r in runs {
        let conc = r.conclusion.trim().to_ascii_lowercase();
        match conc.as_str() {
            "failure" | "timed_out" | "startup_failure" | "action_required" => {
                failed.push(r);
                continue;
            }
            "" => {}
            _ => continue,
        }
        if !conc.is_empty() {
            continue;
        }
        let st = r.status.trim().to_ascii_lowercase();
        if !run_status_in_progress(&st) {
            continue;
        }
        let run = load_run_summary(exec, repo, r.database_id).await?;
        let (_, fail_count, _, fail_jobs) = classify_run_jobs(&run.jobs);
        if fail_count == 0 {
            continue;
        }
        r.conclusion = "failure (run in progress)".into();
        if !fail_jobs.is_empty() {
            r.status = format!("{st}, {fail_count} failed job(s)");
        }
        failed.push(r);
    }
    Ok((head_sha, failed, truncated))
}

pub fn is_failed_job_conclusion(jc: &str) -> bool {
    matches!(
        jc,
        "failure" | "timed_out" | "cancelled" | "startup_failure" | "action_required"
    )
}

pub fn classify_run_jobs(jobs: &[RunJob]) -> (i32, i32, i32, Vec<RunJob>) {
    let mut success = 0i32;
    let mut failed = 0i32;
    let mut pending = 0i32;
    let mut failed_jobs = Vec::new();
    for j in jobs {
        match job_effective_conclusion(j).as_str() {
            "success" | "skipped" | "neutral" => success += 1,
            jc if is_failed_job_conclusion(jc) => {
                failed += 1;
                failed_jobs.push(j.clone());
            }
            _ => pending += 1,
        }
    }
    (success, failed, pending, failed_jobs)
}

pub fn failed_step_names(jobs: &[RunJob]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for j in jobs {
        if !is_failed_job_conclusion(&job_effective_conclusion(j)) {
            continue;
        }
        for step in &j.steps {
            let mut conc = step.conclusion.trim().to_ascii_lowercase();
            if conc.is_empty() {
                conc = step.status.trim().to_ascii_lowercase();
            }
            if conc != "failure" && conc != "timed_out" && conc != "cancelled" {
                continue;
            }
            let name = step.name.trim();
            if name.is_empty() || !seen.insert(name.to_string()) {
                continue;
            }
            out.push(name.to_string());
        }
    }
    out
}

pub fn job_conclusion_skipped(job: &RunJob) -> bool {
    job.conclusion.trim().eq_ignore_ascii_case("skipped")
}

pub fn run_still_in_progress(run: &RunSummary) -> bool {
    let st = run.status.trim().to_ascii_lowercase();
    if run_status_in_progress(&st) {
        return true;
    }
    run.conclusion.trim().is_empty()
}

#[derive(Debug, Clone, Deserialize)]
pub struct PrMergedRow {
    pub number: u32,
    pub title: String,
    pub author: PrAuthor,
    #[serde(rename = "mergedAt")]
    pub merged_at: String,
}

pub async fn default_branch(exec: &GhExec, repo: &str) -> Result<String> {
    let args = [
        "repo",
        "view",
        repo,
        "--json",
        "defaultBranchRef",
        "-q",
        ".defaultBranchRef.name",
    ];
    let res = exec.run_retry(&args).await;
    let branch = GhExec::into_result(res, "failed to resolve default branch")?;
    let branch = branch.trim().to_string();
    if branch.is_empty() {
        return Err(CoworkerError::Other(anyhow::anyhow!(
            "empty default branch for {repo}"
        )));
    }
    Ok(branch)
}

pub async fn list_branch_runs(
    exec: &GhExec,
    repo: &str,
    branch: &str,
    limit: u32,
) -> Result<Vec<BranchRun>> {
    let mut limit = limit;
    if limit == 0 {
        limit = 15;
    }
    if limit > 50 {
        limit = 50;
    }
    let limit_s = limit.to_string();
    let args = [
        "run",
        "list",
        "-R",
        repo,
        "--branch",
        branch,
        "--limit",
        &limit_s,
        "--json",
        "databaseId,workflowName,conclusion,status,headBranch,createdAt,updatedAt",
    ];
    let res = exec.run_retry(&args).await;
    let stdout = GhExec::into_result(res, "failed to list workflow runs")?;
    serde_json::from_str(&stdout)
        .map_err(|e| CoworkerError::Other(anyhow::anyhow!("failed to parse run list: {e}")))
}

pub async fn load_run_summary(exec: &GhExec, repo: &str, run_id: u64) -> Result<RunSummary> {
    let run_s = run_id.to_string();
    let args = [
        "run",
        "view",
        &run_s,
        "-R",
        repo,
        "--json",
        "databaseId,workflowName,status,conclusion,createdAt,updatedAt,headBranch,jobs",
    ];
    let res = exec.run_retry(&args).await;
    let stdout = GhExec::into_result(res, "failed to fetch run summary")?;
    serde_json::from_str(&stdout)
        .map_err(|e| CoworkerError::Other(anyhow::anyhow!("failed to parse run summary: {e}")))
}

pub async fn pr_status_rollup(exec: &GhExec, repo: &str, pr_num: u32) -> Result<Vec<CheckRollup>> {
    let pr_s = pr_num.to_string();
    let args = [
        "pr",
        "view",
        &pr_s,
        "-R",
        repo,
        "--json",
        "statusCheckRollup",
    ];
    let res = exec.run_retry(&args).await;
    let stdout = GhExec::into_result(res, "failed to fetch PR checks")?;
    #[derive(Deserialize)]
    struct Wrapper {
        #[serde(rename = "statusCheckRollup")]
        status_check: Vec<CheckRollup>,
    }
    let wrapper: Wrapper = serde_json::from_str(&stdout)
        .map_err(|e| CoworkerError::Other(anyhow::anyhow!("failed to parse PR checks: {e}")))?;
    Ok(wrapper.status_check)
}

pub fn run_conclusion(r: &BranchRun) -> String {
    let mut c = r.conclusion.trim().to_ascii_lowercase();
    if c.is_empty() {
        c = r.status.trim().to_ascii_lowercase();
    }
    c
}

pub fn is_failed_conclusion(c: &str) -> bool {
    matches!(
        c,
        "failure" | "timed_out" | "cancelled" | "action_required" | "startup_failure" | "stale"
    )
}

pub fn run_status_in_progress(status: &str) -> bool {
    matches!(
        status,
        "queued" | "in_progress" | "pending" | "requested" | "waiting"
    )
}

pub fn run_duration(created: &str, updated: &str, conclusion: &str) -> Duration {
    if run_status_in_progress(conclusion) || created.is_empty() || updated.is_empty() {
        return Duration::ZERO;
    }
    let Ok(t0) = DateTime::parse_from_rfc3339(created) else {
        return Duration::ZERO;
    };
    let Ok(t1) = DateTime::parse_from_rfc3339(updated) else {
        return Duration::ZERO;
    };
    let d = t1.signed_duration_since(t0);
    if d.num_milliseconds() < 0 {
        return Duration::ZERO;
    }
    d.to_std().unwrap_or(Duration::ZERO)
}

pub fn format_run_duration(created: &str, updated: &str, conclusion: &str) -> String {
    if run_status_in_progress(conclusion) || conclusion.is_empty() {
        return "-".into();
    }
    format_duration_compact(run_duration(created, updated, conclusion))
}

pub fn format_duration_compact(d: Duration) -> String {
    if d < Duration::from_secs(60) {
        format!("{}s", d.as_secs())
    } else if d < Duration::from_secs(3600) {
        format!("{}m{}s", d.as_secs() / 60, d.as_secs() % 60)
    } else {
        format!("{}h{}m", d.as_secs() / 3600, (d.as_secs() / 60) % 60)
    }
}

pub fn job_effective_conclusion(j: &RunJob) -> String {
    let mut jc = j.conclusion.trim().to_ascii_lowercase();
    if jc.is_empty() {
        jc = j.status.trim().to_ascii_lowercase();
    }
    jc
}

pub fn job_logs_ready(j: &RunJob) -> bool {
    j.status.trim().eq_ignore_ascii_case("completed")
}

pub fn find_run_job(jobs: &[RunJob], job_id: u64) -> Option<&RunJob> {
    jobs.iter().find(|j| j.database_id == job_id)
}

pub fn short_sha(sha: &str) -> String {
    checks::short_sha(sha)
}

pub fn gh_run_log_recoverable(res: &RunResult) -> bool {
    if res.err.is_none() {
        return res.stdout.trim().is_empty();
    }
    let low = res.combined().to_ascii_lowercase();
    low.contains("still in progress")
        || low.contains("log will be available when it is complete")
        || low.contains("log not found")
}

pub fn tail_bytes(s: &str, n: usize) -> String {
    if s.len() <= n {
        return s.to_string();
    }
    format!("…[truncated]…\n{}", &s[s.len().saturating_sub(n)..])
}

pub fn paginate_lines(
    text: &str,
    offset_lines: usize,
    max_lines: usize,
) -> (String, usize, usize, bool) {
    if max_lines == 0 {
        return (text.to_string(), 0, 0, false);
    }
    let lines: Vec<&str> = text.split('\n').collect();
    let total = lines.len();
    if offset_lines >= total {
        return (String::new(), total, total, false);
    }
    let end = (offset_lines + max_lines).min(total);
    let page = lines[offset_lines..end].join("\n");
    let next = end;
    let has_more = end < total;
    (page, total, next, has_more)
}

pub fn parse_rfc3339(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.with_timezone(&Utc))
}
