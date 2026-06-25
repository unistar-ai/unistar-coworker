use chrono::{Duration as ChronoDuration, Utc};
use serde::Deserialize;
use serde_json::Value;

use super::args::{optional_u32, require_str, require_u64};
use super::ci_common::{self, PrMergedRow};
use super::ci_logs;
use super::exec::GhExec;
use crate::error::{CoworkerError, Result};

#[derive(Debug, Deserialize)]
struct WorkflowRow {
    id: u64,
    name: String,
    state: String,
}

#[derive(Debug, Deserialize)]
struct RunCorrelateMeta {
    #[serde(rename = "databaseId")]
    database_id: u64,
    #[serde(rename = "workflowName")]
    workflow_name: String,
    #[serde(rename = "headBranch")]
    head_branch: String,
    #[serde(rename = "headSha")]
    head_sha: String,
    #[serde(rename = "createdAt")]
    created_at: String,
    conclusion: String,
    status: String,
}

pub async fn ci_list_workflows(exec: &GhExec, args: &Value) -> Result<String> {
    let repo = require_str(args, "repo")?;
    let mut limit = optional_u32(args, "limit", 30);
    if limit == 0 {
        limit = 30;
    }
    if limit > 100 {
        limit = 100;
    }
    let limit_s = limit.to_string();
    let gh_args = [
        "workflow",
        "list",
        "-R",
        &repo,
        "--limit",
        &limit_s,
        "--json",
        "id,name,state",
    ];
    let res = exec.run_retry(&gh_args).await;
    let stdout = GhExec::into_result(res, "failed to list workflows")?;
    let mut workflows: Vec<WorkflowRow> = serde_json::from_str(&stdout)
        .map_err(|e| CoworkerError::Other(anyhow::anyhow!("failed to parse workflow list: {e}")))?;
    workflows.sort_by(|a, b| {
        a.name
            .to_ascii_lowercase()
            .cmp(&b.name.to_ascii_lowercase())
    });
    if workflows.is_empty() {
        return Ok(format!("No workflows found for {repo}."));
    }
    let mut lines = vec![format!("{} workflow(s) for {repo}:", workflows.len())];
    for wf in &workflows {
        let state = wf.state.trim().to_ascii_lowercase();
        let state = if state.is_empty() { "unknown" } else { &state };
        lines.push(format!("{}  {}  {state}", wf.id, wf.name));
    }
    lines.push("Next: ci_list_runs or ci_branch_health on the default branch.".into());
    Ok(lines.join("\n"))
}

pub async fn ci_get_job_logs(exec: &GhExec, args: &Value) -> Result<String> {
    let repo = require_str(args, "repo")?;
    let run_id = require_u64(args, "run_id")?;
    let job_id = require_u64(args, "job_id")?;
    let offset_lines = args
        .get("offset_lines")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;
    let max_lines = args.get("max_lines").and_then(|v| v.as_u64()).unwrap_or(0) as usize;

    let run = ci_common::load_run_summary(exec, &repo, run_id).await?;
    let job = ci_common::find_run_job(&run.jobs, job_id).ok_or_else(|| {
        CoworkerError::Other(anyhow::anyhow!("job_id {job_id} not found in run {run_id}"))
    })?;

    let raw = ci_logs::fetch_job_log_text(exec, &repo, run_id, job, true).await?;
    if raw.trim().is_empty() {
        let mut state = ci_common::job_effective_conclusion(job);
        if !ci_common::job_logs_ready(job) {
            state.push_str(", logs pending");
        }
        return Ok(format!(
            "Job {} (job_id={job_id}) in run {run_id} has no log output yet ({state}).",
            job.name
        ));
    }
    Ok(ci_logs::format_distilled_job_logs(
        run_id,
        job_id,
        &job.name,
        &raw,
        offset_lines,
        max_lines,
    ))
}

pub async fn ci_correlate_prs(exec: &GhExec, args: &Value) -> Result<String> {
    let repo = require_str(args, "repo")?;
    let run_id = require_u64(args, "run_id")?;
    let mut window_days = optional_u32(args, "window_days", 7);
    if window_days == 0 {
        window_days = 7;
    }
    if window_days > 30 {
        window_days = 30;
    }
    let mut limit = optional_u32(args, "limit", 10);
    if limit == 0 {
        limit = 10;
    }
    if limit > 30 {
        limit = 30;
    }

    let meta = load_run_correlate_meta(exec, &repo, run_id).await?;
    let run_at = ci_common::parse_rfc3339(&meta.created_at).ok_or_else(|| {
        CoworkerError::Other(anyhow::anyhow!(
            "failed to parse run createdAt {:?}",
            meta.created_at
        ))
    })?;
    let since_date = (run_at - ChronoDuration::days(window_days as i64))
        .format("%Y-%m-%d")
        .to_string();
    let mut branch = meta.head_branch.trim().to_string();
    if branch.is_empty() {
        branch = ci_common::default_branch(exec, &repo).await?;
    }

    let prs = merged_prs_on_branch(exec, &repo, &branch, &since_date, limit * 3).await?;
    let suspects = filter_prs_before_run(&prs, run_at, limit);

    let mut conclusion = meta.conclusion.trim().to_ascii_lowercase();
    if conclusion.is_empty() {
        conclusion = meta.status.trim().to_ascii_lowercase();
    }

    let mut lines = vec![format!(
        "Run {} ({}) on {} @{} — {}",
        meta.database_id,
        meta.workflow_name,
        branch,
        ci_common::short_sha(&meta.head_sha),
        conclusion
    )];
    lines.push(format!(
        "Merged PRs on {branch} in {window_days} days before run ({since_date}):"
    ));
    if suspects.is_empty() {
        lines.push("(none in window)".into());
    } else {
        for pr in &suspects {
            let merged = if pr.merged_at.len() >= 10 {
                &pr.merged_at[..10]
            } else {
                &pr.merged_at
            };
            lines.push(format!(
                "#{}  {}  @{}  merged:{merged}",
                pr.number, pr.title, pr.author.login
            ));
        }
    }
    lines.push("Next: pr_get_overview on top rows or ci_compare_runs with last green run.".into());
    Ok(lines.join("\n"))
}

async fn load_run_correlate_meta(
    exec: &GhExec,
    repo: &str,
    run_id: u64,
) -> Result<RunCorrelateMeta> {
    let run_s = run_id.to_string();
    let args = [
        "run",
        "view",
        &run_s,
        "-R",
        repo,
        "--json",
        "databaseId,workflowName,headBranch,headSha,createdAt,conclusion,status",
    ];
    let res = exec.run_retry(&args).await;
    let stdout = GhExec::into_result(res, "failed to fetch run metadata")?;
    serde_json::from_str(&stdout)
        .map_err(|e| CoworkerError::Other(anyhow::anyhow!("failed to parse run metadata: {e}")))
}

async fn merged_prs_on_branch(
    exec: &GhExec,
    repo: &str,
    branch: &str,
    since_date: &str,
    fetch_limit: u32,
) -> Result<Vec<PrMergedRow>> {
    let mut fetch_limit = fetch_limit;
    if fetch_limit == 0 {
        fetch_limit = 30;
    }
    let search = format!("merged:>={since_date} base:{branch}");
    let limit_s = fetch_limit.to_string();
    let gh_args = [
        "pr",
        "list",
        "-R",
        repo,
        "--state",
        "merged",
        "--limit",
        &limit_s,
        "--json",
        "number,title,author,mergedAt",
        "--search",
        &search,
    ];
    let res = exec.run_retry(&gh_args).await;
    let stdout = GhExec::into_result(res, "failed to list merged PRs")?;
    let mut prs: Vec<PrMergedRow> = serde_json::from_str(&stdout).map_err(|e| {
        CoworkerError::Other(anyhow::anyhow!("failed to parse merged PR list: {e}"))
    })?;
    prs.sort_by(|a, b| b.merged_at.cmp(&a.merged_at));
    Ok(prs)
}

fn filter_prs_before_run(
    prs: &[PrMergedRow],
    run_at: chrono::DateTime<Utc>,
    limit: u32,
) -> Vec<PrMergedRow> {
    let mut out = Vec::new();
    for pr in prs {
        let Some(merged_at) = ci_common::parse_rfc3339(&pr.merged_at) else {
            continue;
        };
        if !merged_at.lt(&run_at) {
            continue;
        }
        out.push(pr.clone());
        if out.len() >= limit as usize {
            break;
        }
    }
    out
}
