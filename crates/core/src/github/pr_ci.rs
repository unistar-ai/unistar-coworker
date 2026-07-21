//! Minimal CI helpers for PR overview and draft-comment tools.

use regex::Regex;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::exec::{GhExec, RunResult};
use crate::error::{CoworkerError, Result};

const CI_RUN_LIST_LIMIT: usize = 100;

#[derive(Debug, Deserialize)]
struct WorkflowRun {
    #[serde(rename = "databaseId")]
    database_id: i64,
    #[serde(rename = "workflowName")]
    workflow_name: String,
    conclusion: String,
    status: String,
}

#[derive(Debug, Clone)]
pub struct FailedWorkflowRun {
    pub database_id: i64,
    pub workflow_name: String,
    pub conclusion: String,
}

#[derive(Debug, Clone, Deserialize)]
struct RunStep {
    name: String,
    status: String,
    conclusion: String,
}

#[derive(Debug, Deserialize)]
pub struct RunJob {
    #[serde(rename = "databaseId")]
    database_id: i64,
    name: String,
    status: String,
    conclusion: String,
    #[serde(default)]
    steps: Vec<RunStep>,
}

#[derive(Debug, Deserialize)]
pub struct RunSummary {
    #[serde(rename = "workflowName")]
    workflow_name: String,
    #[serde(default)]
    jobs: Vec<RunJob>,
}

#[derive(Debug, Clone)]
pub struct RunFailureAnalysis {
    pub run_id: i64,
    pub workflow: String,
    pub job: String,
    pub step: String,
    pub test_name: String,
    pub error_sig: String,
    pub fingerprint: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureVerdict {
    Test,
    Infra,
    Auth,
    Timeout,
    External,
    Unknown,
}

impl FailureVerdict {
    fn as_str(self) -> &'static str {
        match self {
            Self::Test => "test",
            Self::Infra => "infra",
            Self::Auth => "auth",
            Self::Timeout => "timeout",
            Self::External => "external_ci",
            Self::Unknown => "unknown",
        }
    }
}

pub async fn pr_head_sha(exec: &GhExec, repo: &str, pr_num: u32) -> Result<String> {
    let pr_num_s = pr_num.to_string();
    let args = [
        "pr",
        "view",
        &pr_num_s,
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
) -> Result<(String, Vec<FailedWorkflowRun>)> {
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
    let runs: Vec<WorkflowRun> = serde_json::from_str(&stdout).map_err(|e| {
        CoworkerError::Other(anyhow::anyhow!(super::error::format_tool_error(
            super::error::ErrCode::Generic,
            &format!("failed to parse run list: {e}"),
            "retry",
        )))
    })?;

    let mut failed = Vec::new();
    for mut r in runs {
        let conc = r.conclusion.trim().to_ascii_lowercase();
        match conc.as_str() {
            "failure" | "timed_out" | "startup_failure" | "action_required" => {
                failed.push(FailedWorkflowRun {
                    database_id: r.database_id,
                    workflow_name: r.workflow_name,
                    conclusion: r.conclusion,
                });
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
        let (_, fail_count, _, _) = classify_run_jobs(&run.jobs);
        if fail_count == 0 {
            continue;
        }
        r.conclusion = "failure (run in progress)".into();
        failed.push(FailedWorkflowRun {
            database_id: r.database_id,
            workflow_name: r.workflow_name,
            conclusion: r.conclusion,
        });
    }
    Ok((head_sha, failed))
}

pub async fn analyze_run_failure(
    exec: &GhExec,
    repo: &str,
    run_id: i64,
) -> Result<RunFailureAnalysis> {
    let run = load_run_summary(exec, repo, run_id).await?;
    let (raw_logs, failed_jobs) = fetch_failed_run_logs(exec, repo, run_id).await?;

    let job = failed_jobs
        .first()
        .map(|j| j.name.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            let (_, _, _, fj) = classify_run_jobs(&run.jobs);
            fj.first().map(|j| j.name.trim().to_string())
        })
        .unwrap_or_default();

    let step = failed_step_names(&run.jobs)
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

pub fn classify_failure(analysis: &RunFailureAnalysis) -> (FailureVerdict, &'static str) {
    let corpus = format!(
        "{} {} {} {} {}",
        analysis.error_sig, analysis.test_name, analysis.job, analysis.step, analysis.workflow
    )
    .to_ascii_lowercase();

    const RULES: &[(&str, FailureVerdict, &[&str])] = &[
        (
            "external_ci_hint",
            FailureVerdict::External,
            &[
                "external ci",
                "status context",
                "jenkins",
                "codecov",
                "third-party check",
            ],
        ),
        (
            "timeout",
            FailureVerdict::Timeout,
            &[
                "timeout",
                "timed out",
                "deadline exceeded",
                "context deadline",
                "i/o timeout",
            ],
        ),
        (
            "auth",
            FailureVerdict::Auth,
            &[
                "401",
                "403",
                "unauthorized",
                "authentication failed",
                "permission denied",
                "bad credentials",
                "invalid token",
                "access denied",
            ],
        ),
        (
            "infra",
            FailureVerdict::Infra,
            &[
                "connection refused",
                "connection reset",
                "no space left",
                "out of memory",
                "oom",
                "docker",
                "registry unreachable",
                "503 service unavailable",
                "502 bad gateway",
                "504 gateway",
                "network is unreachable",
                "cannot connect",
                "runner lost communication",
                "pod evicted",
            ],
        ),
    ];

    for (id, verdict, subs) in RULES {
        for sub in *subs {
            if corpus.contains(sub) {
                return (*verdict, id);
            }
        }
    }

    if !analysis.test_name.trim().is_empty() {
        return (FailureVerdict::Test, "named_test_failure");
    }

    let low = analysis.error_sig.to_ascii_lowercase();
    if low.contains("assert")
        || low.contains("expect")
        || low.contains("panic:")
        || low.contains("failed:")
    {
        return (FailureVerdict::Test, "test_assertion");
    }

    (FailureVerdict::Unknown, "no_rule_match")
}

pub fn format_draft_ci_comment(
    repo: &str,
    pr_num: u32,
    analysis: &RunFailureAnalysis,
    verdict: FailureVerdict,
    rule_id: &str,
) -> String {
    let mut out = String::from("DRAFT COMMENT (edit before pr_post_comment):\n\n");
    out.push_str(&format!("### CI failure on run {}\n", analysis.run_id));
    out.push_str(&format!("Repo: {repo}  PR: #{pr_num}\n"));
    out.push_str(&format!("Workflow: **{}**", analysis.workflow));
    if !analysis.job.is_empty() {
        out.push_str(&format!("  Job: **{}**", analysis.job));
    }
    out.push('\n');
    if !analysis.step.is_empty() {
        out.push_str(&format!("Failed step: {}\n", analysis.step));
    }
    if !analysis.test_name.is_empty() {
        out.push_str(&format!("Test: `{}`\n", analysis.test_name));
    }
    if !analysis.error_sig.is_empty() {
        out.push_str(&format!("Error: {}\n", analysis.error_sig));
    }
    out.push_str(&format!(
        "Policy: **{}** (rule: {rule_id})\n",
        verdict.as_str()
    ));
    out.push_str(&format!("Fingerprint: `{}`\n", analysis.fingerprint));
    out.push('\n');
    match verdict {
        FailureVerdict::Timeout | FailureVerdict::Infra => {
            out.push_str("Looks like a transient infra/timeout failure — consider rerunning CI if this fingerprint is new.");
        }
        FailureVerdict::Auth => {
            out.push_str(
                "Auth/permission failure — please check secrets or token scopes before rerunning.",
            );
        }
        FailureVerdict::Test => {
            out.push_str("Test failure — please investigate the failing test before merge.");
        }
        _ => {
            out.push_str("Please investigate the linked workflow run logs.");
        }
    }
    out.push_str("\n\nNext: pr_post_comment with edited body (approval required).");
    out.trim().to_string()
}

pub async fn load_run_summary(exec: &GhExec, repo: &str, run_id: i64) -> Result<RunSummary> {
    let run_id_s = run_id.to_string();
    let args = [
        "run",
        "view",
        &run_id_s,
        "-R",
        repo,
        "--json",
        "workflowName,jobs",
    ];
    let res = exec.run_retry(&args).await;
    let stdout = GhExec::into_result(res, "failed to fetch run summary")?;
    serde_json::from_str(&stdout).map_err(|e| {
        CoworkerError::Other(anyhow::anyhow!(super::error::format_tool_error(
            super::error::ErrCode::Generic,
            &format!("failed to parse run summary: {e}"),
            "retry",
        )))
    })
}

fn classify_run_jobs(jobs: &[RunJob]) -> (i32, i32, i32, Vec<RunJob>) {
    let mut success = 0i32;
    let mut failed = 0i32;
    let mut pending = 0i32;
    let mut failed_jobs = Vec::new();
    for j in jobs {
        match job_effective_conclusion(j).as_str() {
            "success" | "skipped" | "neutral" => success += 1,
            "failure" | "timed_out" | "cancelled" | "startup_failure" | "action_required" => {
                failed += 1;
                failed_jobs.push(RunJob {
                    database_id: j.database_id,
                    name: j.name.clone(),
                    status: j.status.clone(),
                    conclusion: j.conclusion.clone(),
                    steps: j.steps.clone(),
                });
            }
            _ => pending += 1,
        }
    }
    (success, failed, pending, failed_jobs)
}

fn job_effective_conclusion(j: &RunJob) -> String {
    let jc = j.conclusion.trim().to_ascii_lowercase();
    if jc.is_empty() {
        j.status.trim().to_ascii_lowercase()
    } else {
        jc
    }
}

fn failed_step_names(jobs: &[RunJob]) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for j in jobs {
        if !matches!(
            job_effective_conclusion(j).as_str(),
            "failure" | "timed_out" | "cancelled"
        ) {
            continue;
        }
        for step in &j.steps {
            let conc = if step.conclusion.trim().is_empty() {
                step.status.trim().to_ascii_lowercase()
            } else {
                step.conclusion.trim().to_ascii_lowercase()
            };
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

fn run_status_in_progress(status: &str) -> bool {
    matches!(
        status,
        "queued" | "in_progress" | "pending" | "requested" | "waiting"
    )
}

async fn fetch_failed_run_logs(
    exec: &GhExec,
    repo: &str,
    run_id: i64,
) -> Result<(String, Vec<RunJob>)> {
    let run_id_s = run_id.to_string();
    let args = ["run", "view", &run_id_s, "-R", repo, "--log-failed"];
    let res = exec.run_retry(&args).await;
    if !res.stdout.trim().is_empty() {
        return Ok((res.stdout, Vec::new()));
    }
    if res.err.is_some() && !gh_run_log_recoverable(&res) {
        return Err(res.wrap("failed to fetch failed logs"));
    }
    fetch_failed_job_logs(exec, repo, run_id).await
}

async fn fetch_failed_job_logs(
    exec: &GhExec,
    repo: &str,
    run_id: i64,
) -> Result<(String, Vec<RunJob>)> {
    let run = load_run_summary(exec, repo, run_id).await?;
    let (_, _, _, failed_jobs) = classify_run_jobs(&run.jobs);
    if failed_jobs.is_empty() {
        return Ok((String::new(), Vec::new()));
    }

    let mut parts = Vec::new();
    for job in &failed_jobs {
        match fetch_failed_job_log_text(exec, repo, run_id, job).await {
            Ok(raw) if !raw.trim().is_empty() => {
                parts.push(format!(
                    "=== job: {} (job_id={}) ===\n{}",
                    job.name,
                    job.database_id,
                    raw.trim()
                ));
            }
            Ok(_) => {}
            Err(e) => {
                parts.push(format!(
                    "=== job: {} (job_id={}) ===\nfailed to fetch logs: {e}",
                    job.name, job.database_id
                ));
            }
        }
    }
    Ok((parts.join("\n\n"), failed_jobs))
}

async fn fetch_failed_job_log_text(
    exec: &GhExec,
    repo: &str,
    run_id: i64,
    job: &RunJob,
) -> Result<String> {
    if job.conclusion.trim().eq_ignore_ascii_case("skipped") {
        return Ok(String::new());
    }
    let job_id_s = job.database_id.to_string();
    let run_id_s = run_id.to_string();
    let attempts: [&[&str]; 4] = [
        &[
            "run",
            "view",
            "-R",
            repo,
            "--job",
            &job_id_s,
            "--log-failed",
        ],
        &[
            "run",
            "view",
            &run_id_s,
            "-R",
            repo,
            "--job",
            &job_id_s,
            "--log-failed",
        ],
        &["run", "view", "-R", repo, "--job", &job_id_s, "--log"],
        &[
            "run", "view", &run_id_s, "-R", repo, "--job", &job_id_s, "--log",
        ],
    ];
    for args in attempts {
        let res = exec.run_retry(args).await;
        if !res.stdout.trim().is_empty() {
            return Ok(res.stdout);
        }
        if res.err.is_some() && !gh_run_log_recoverable(&res) {
            return Err(res.wrap(&format!("fetch logs for job {}", job.database_id)));
        }
    }
    if !job.status.trim().eq_ignore_ascii_case("completed") {
        return Ok(String::new());
    }
    let path = format!("repos/{repo}/actions/jobs/{}/logs", job.database_id);
    let args = ["api", &path];
    let res = exec.run_retry(&args).await;
    if !res.stdout.trim().is_empty() {
        return Ok(res.stdout);
    }
    if res.err.is_some() && !gh_run_log_recoverable(&res) {
        return Err(res.wrap(&format!("fetch logs for job {}", job.database_id)));
    }
    Ok(String::new())
}

fn gh_run_log_recoverable(res: &RunResult) -> bool {
    if res.err.is_none() {
        return res.stdout.trim().is_empty();
    }
    if gh_run_log_unavailable_yet(res) {
        return true;
    }
    res.combined()
        .to_ascii_lowercase()
        .contains("log not found")
}

fn gh_run_log_unavailable_yet(res: &RunResult) -> bool {
    if res.err.is_none() {
        return res.stdout.trim().is_empty();
    }
    let low = res.combined().to_ascii_lowercase();
    low.contains("still in progress") || low.contains("log will be available when it is complete")
}

fn compute_failure_fingerprint(
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
    hash.iter().map(|b| format!("{b:02x}")).collect()
}

fn extract_test_name_from_logs(logs: &str) -> String {
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

fn extract_error_signature(logs: &str) -> String {
    let clean = clean_gh_log(logs);
    let body = extract_errors_simple(&clean);
    let body = if body.trim().is_empty() {
        tail_bytes(&clean, 500)
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

fn extract_errors_simple(clean: &str) -> String {
    static ERR_LINE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let re = ERR_LINE.get_or_init(|| {
        Regex::new(
            r"(?i)(\berror\b|\bfailed\b|\bfailure\b|\bpanic\b|\bfatal\b|exception|traceback|assert|\bundefined\b|cannot |not found|exit code [1-9]|exit status [1-9]|✗|\bFAIL\b|\[error\])",
        )
        .unwrap()
    });
    let mut matched = Vec::new();
    for line in clean.lines() {
        if re.is_match(line) && !is_noise_error_line(line) {
            matched.push(line.trim());
        }
    }
    matched.join("\n")
}

fn is_noise_error_line(ln: &str) -> bool {
    let low = ln.to_ascii_lowercase();
    if ln.contains("##[warning]") {
        return true;
    }
    if low.contains("unable to reserve cache") {
        return true;
    }
    if low.contains("failed to save:") && low.contains("cache") {
        return true;
    }
    if low.contains("npm warn") {
        return true;
    }
    if low.contains("warning:") && !ln.contains("##[error]") {
        return true;
    }
    if low.contains("retrying") || low.contains("attempt 2 of") || low.contains("attempt 3 of") {
        return true;
    }
    if low.contains("downloading") && (low.contains("mb/") || low.contains("mb ")) {
        return true;
    }
    low.contains("uploaded artifact")
}

fn clean_gh_log(s: &str) -> String {
    static ANSI: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    static RAW: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let ansi = ANSI.get_or_init(|| Regex::new(r"\x1b\[[0-9;]*[a-zA-Z]").unwrap());
    let raw = RAW.get_or_init(|| Regex::new(r"^\d{4}-\d{2}-\d{2}T[\d:.]+Z\s*").unwrap());
    let mut out = String::new();
    for line in s.lines() {
        let line = ansi.replace_all(line, "");
        let line = raw.replace(&line, "");
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}

fn tail_bytes(s: &str, limit: usize) -> String {
    if s.len() <= limit {
        return s.to_string();
    }
    s[s.len().saturating_sub(limit)..].to_string()
}

fn truncate_runes(s: &str, max: usize) -> String {
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

pub fn clip_for_log(s: &str, limit: usize) -> String {
    if s.len() <= limit {
        s.to_string()
    } else {
        format!("{}…[truncated]", &s[..limit])
    }
}
