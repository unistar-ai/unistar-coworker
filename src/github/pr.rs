use std::collections::HashSet;
use std::path::{Path, MAIN_SEPARATOR};

use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use serde_json::Value;

use super::args::{optional_str, optional_u32, require_i64, require_str, require_u32};
use super::checks::{
    check_display_name, check_verdict, ci_state, format_external_check_summary, mergeable_state,
    review_state, short_sha, tally_checks, CheckRollup,
};
use super::error::format_tool_ok;
use super::exec::GhExec;
use super::pr_ci::{
    analyze_run_failure, classify_failure, clip_for_log, failing_runs_for_pr,
    format_draft_ci_comment,
};
use crate::error::{CoworkerError, Result};

const DEFAULT_PR_LIST_LIMIT: u32 = 20;
const DEFAULT_BACKPORT_LABEL: &str = "needs-backport";
const MAX_REVIEW_COMMENTS: usize = 20;

#[derive(Debug, Default, Deserialize)]
pub(crate) struct PrAuthor {
    #[serde(default)]
    login: String,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct PullRequest {
    pub(crate) number: u32,
    pub(crate) title: String,
    author: PrAuthor,
    #[serde(default)]
    state: String,
    #[serde(default, rename = "isDraft")]
    is_draft: bool,
    #[serde(default, rename = "mergeable")]
    mergeable: String,
    #[serde(default, rename = "reviewDecision")]
    review_decision: String,
    #[serde(default, rename = "statusCheckRollup")]
    status_check: Vec<CheckRollup>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct PullRequestOverviewBatch {
    pub(crate) number: u32,
    pub(crate) title: String,
    author: PrAuthor,
    #[serde(default, rename = "isDraft")]
    is_draft: bool,
    #[serde(default, rename = "reviewDecision")]
    review_decision: String,
    #[serde(default, rename = "statusCheckRollup")]
    status_check: Vec<CheckRollup>,
    #[serde(default)]
    additions: i32,
    #[serde(default)]
    deletions: i32,
    #[serde(default, rename = "changedFiles")]
    changed_files: i32,
}

#[derive(Debug, Deserialize)]
struct PrFileChange {
    filename: String,
    additions: i32,
    deletions: i32,
    status: String,
}

#[derive(Debug, Deserialize)]
struct PrFilePatchRow {
    filename: String,
    additions: i32,
    deletions: i32,
    status: String,
    #[serde(default)]
    patch: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PrUpdatedRow {
    number: u32,
    title: String,
    author: PrAuthor,
    #[serde(default, rename = "isDraft")]
    is_draft: bool,
    #[serde(rename = "updatedAt")]
    updated_at: String,
}

#[derive(Debug, Deserialize)]
struct PrMergedRow {
    number: u32,
    title: String,
    author: PrAuthor,
    #[serde(rename = "mergedAt")]
    merged_at: String,
}

#[derive(Debug, Deserialize)]
struct PrSizeRow {
    number: u32,
    title: String,
    author: PrAuthor,
    additions: i32,
    deletions: i32,
    #[serde(rename = "changedFiles")]
    changed_files: i32,
}

#[derive(Debug, Deserialize)]
struct ReviewRequest {
    login: String,
}

#[derive(Debug, Deserialize)]
struct LatestReview {
    author: PrAuthor,
    state: String,
    body: String,
}

#[derive(Debug, Deserialize)]
struct PrReviewView {
    number: u32,
    title: String,
    #[serde(rename = "reviewDecision")]
    review_decision: String,
    #[serde(default, rename = "reviewRequests")]
    review_requests: Vec<ReviewRequest>,
    #[serde(default, rename = "latestReviews")]
    latest_reviews: Vec<LatestReview>,
}

#[derive(Debug, Deserialize)]
struct InlineComment {
    path: String,
    line: i32,
    user: PrAuthor,
    body: String,
}

#[derive(Debug, Deserialize)]
struct CodeownersRule {
    pattern: String,
    owners: Vec<String>,
}

// --- existing tools ---

pub async fn pr_list_open(exec: &GhExec, args: &Value) -> Result<String> {
    let repo = require_str(args, "repo")?;
    let author = optional_str(args, "author");
    let limit = optional_u32(args, "limit", DEFAULT_PR_LIST_LIMIT);
    let limit_s = limit.to_string();
    let mut gh_args = vec![
        "pr",
        "list",
        "-R",
        &repo,
        "--state",
        "open",
        "--limit",
        &limit_s,
        "--json",
        "number,title,author,isDraft,reviewDecision,statusCheckRollup",
    ];
    if let Some(ref a) = author {
        gh_args.push("--author");
        gh_args.push(a);
    }
    let res = exec.run_retry(&gh_args).await;
    let stdout = GhExec::into_result(res, "failed to list pull requests")?;
    let prs: Vec<PullRequest> = serde_json::from_str(&stdout).map_err(|e| {
        CoworkerError::Other(anyhow::anyhow!(super::error::format_tool_error(
            super::error::ErrCode::Generic,
            &format!("failed to parse PR list: {e}"),
            "retry or check gh output",
        )))
    })?;
    if prs.is_empty() {
        return Ok(if let Some(ref author) = author {
            format!("No open PRs by {author} in {repo}.")
        } else {
            format!("No open PRs in {repo}.")
        });
    }
    let mut lines = vec![
        format!("{} open PR(s) in {repo}:", prs.len()),
        format!("(list may be truncated at limit={limit}; pass a larger limit to see more)"),
    ];
    for pr in &prs {
        lines.push(format_pr_list_line(pr));
    }
    Ok(lines.join("\n"))
}

pub async fn pr_get_status(exec: &GhExec, args: &Value) -> Result<String> {
    let repo = require_str(args, "repo")?;
    let pr_num = require_u32(args, "pr_number")?;
    let pr = fetch_pr_status(exec, &repo, pr_num).await?;
    Ok(format_pr_status(&pr))
}

pub async fn pr_list_changed_files(exec: &GhExec, args: &Value) -> Result<String> {
    let repo = require_str(args, "repo")?;
    let pr_num = require_u32(args, "pr_number")?;
    let files = fetch_pr_file_changes(exec, &repo, pr_num).await?;
    format_changed_files_list(&repo, pr_num, &files)
}

pub async fn pr_get_overview(exec: &GhExec, args: &Value) -> Result<String> {
    let repo = require_str(args, "repo")?;
    let pr_num = require_u32(args, "pr_number")?;
    let pr = fetch_pr_overview(exec, &repo, pr_num).await?;
    build_proverview_text(exec, &repo, pr_num, &pr).await
}

// --- pr.go extensions ---

pub async fn pr_list_stale(exec: &GhExec, args: &Value) -> Result<String> {
    let repo = require_str(args, "repo")?;
    let days = optional_u32(args, "days", 7);
    let limit = optional_u32(args, "limit", DEFAULT_PR_LIST_LIMIT);
    let cutoff = Utc::now() - Duration::days(days as i64);

    let gh_args = [
        "pr",
        "list",
        "-R",
        &repo,
        "--state",
        "open",
        "--limit",
        "100",
        "--json",
        "number,title,author,isDraft,updatedAt",
    ];
    let res = exec.run_retry(&gh_args).await;
    let stdout = GhExec::into_result(res, "failed to list pull requests")?;
    let prs: Vec<PrUpdatedRow> = serde_json::from_str(&stdout).map_err(|e| {
        CoworkerError::Other(anyhow::anyhow!(super::error::format_tool_error(
            super::error::ErrCode::Generic,
            &format!("failed to parse PR list: {e}"),
            "retry",
        )))
    })?;

    let mut stale = Vec::new();
    for pr in prs {
        if pr.is_draft {
            continue;
        }
        let Ok(updated) = DateTime::parse_from_rfc3339(&pr.updated_at) else {
            continue;
        };
        if updated < cutoff {
            stale.push(pr);
        }
    }
    if stale.is_empty() {
        return Ok(format!(
            "No stale open PRs (>{days}d without update) in {repo}."
        ));
    }
    if stale.len() > limit as usize {
        stale.truncate(limit as usize);
    }

    let mut lines = vec![format!(
        "{} stale open PR(s) in {repo} (no update in {days}+ days):",
        stale.len()
    )];
    for pr in &stale {
        let date = pr.updated_at.get(..10).unwrap_or(&pr.updated_at);
        lines.push(format!(
            "#{}  {}  @{}  updated:{date}",
            pr.number, pr.title, pr.author.login
        ));
    }
    Ok(lines.join("\n"))
}

pub async fn pr_list_merged(exec: &GhExec, args: &Value) -> Result<String> {
    let repo = require_str(args, "repo")?;
    let since_raw = optional_str(args, "since").unwrap_or_default();
    let limit = optional_u32(args, "limit", 30);
    let label = optional_str(args, "label").unwrap_or_default();
    format_merged_pr_list(exec, &repo, &since_raw, &label, limit, false).await
}

pub async fn pr_get_diff(exec: &GhExec, args: &Value) -> Result<String> {
    let repo = require_str(args, "repo")?;
    let pr_num = require_u32(args, "pr_number")?;
    let max_bytes = optional_u32(args, "max_bytes", 48_000) as usize;
    if let Some(path) = optional_str(args, "path") {
        return pr_get_diff_for_path(exec, &repo, pr_num, &path, max_bytes).await;
    }
    let pr_num_s = pr_num.to_string();
    let gh_args = ["pr", "diff", &pr_num_s, "-R", &repo];
    let res = exec.run_retry(&gh_args).await;
    let mut diff = GhExec::into_result(res, "failed to fetch PR diff")?;
    let truncated = diff.len() > max_bytes;
    if truncated {
        diff.truncate(max_bytes);
    }
    let mut out = format!("Diff for {repo}#{pr_num} ({} bytes", diff.len());
    if truncated {
        out.push_str(", truncated");
    }
    out.push_str("):\n\n");
    out.push_str(&diff);
    if truncated {
        out.push_str("\n\n[diff truncated at max_bytes]");
        out.push_str("\nNext: pr_list_changed_files, then pr_get_diff with path=<file> per file.");
    }
    Ok(out)
}

async fn pr_get_diff_for_path(
    exec: &GhExec,
    repo: &str,
    pr_num: u32,
    path: &str,
    max_bytes: usize,
) -> Result<String> {
    let file = fetch_pr_file_patch(exec, repo, pr_num, path)
        .await?
        .ok_or_else(|| {
            CoworkerError::Workflow(format!(
                "path not in PR #{pr_num} changed files: {path}\n\
Next: pr_list_changed_files for exact paths."
            ))
        })?;
    let patch = match file.patch.filter(|p| !p.trim().is_empty()) {
        Some(patch) => patch,
        None => {
            return Ok(format!(
                "Diff for {repo}#{pr_num} path={path}:\n\n\
No patch available (status={}, +{}/-{}). File may be binary or exceed GitHub's per-file patch limit.\n\
Next: pr_list_changed_files; use read_file in workspace if the branch is checked out.",
                file.status, file.additions, file.deletions
            ));
        }
    };
    let mut diff = format!("diff --git a/{path} b/{path}\n{patch}");
    let truncated = diff.len() > max_bytes;
    if truncated {
        diff.truncate(max_bytes);
    }
    let mut out = format!("Diff for {repo}#{pr_num} path={path} ({} bytes", diff.len());
    if truncated {
        out.push_str(", truncated");
    }
    out.push_str("):\n\n");
    out.push_str(&diff);
    if truncated {
        out.push_str("\n\n[diff truncated at max_bytes]");
    }
    Ok(out)
}

pub async fn pr_post_comment(exec: &GhExec, args: &Value) -> Result<String> {
    let repo = require_str(args, "repo")?;
    let pr_num = require_u32(args, "pr_number")?;
    let body = require_str(args, "body")?;
    let pr_num_s = pr_num.to_string();
    let gh_args = ["pr", "comment", &pr_num_s, "-R", &repo, "--body", &body];
    let res = exec.run(&gh_args).await;
    GhExec::into_result(res, "failed to post PR comment")?;
    Ok(format_tool_ok(&format!(
        "Comment posted on {repo}#{pr_num}."
    )))
}

// --- pr_chat_tools.go ---

pub async fn pr_get_merge_blockers(exec: &GhExec, args: &Value) -> Result<String> {
    let repo = require_str(args, "repo")?;
    let pr_num = require_u32(args, "pr_number")?;
    build_pr_merge_blockers_text(exec, &repo, pr_num).await
}

pub async fn pr_list_waiting_review(exec: &GhExec, args: &Value) -> Result<String> {
    let repo = require_str(args, "repo")?;
    let limit = optional_u32(args, "limit", DEFAULT_PR_LIST_LIMIT);
    let fetch_limit = (limit * 3).min(100).max(limit);
    let fetch_limit_s = fetch_limit.to_string();

    let gh_args = [
        "search",
        "prs",
        "--repo",
        &repo,
        "is:pr is:open review:required",
        "--limit",
        &fetch_limit_s,
        "--json",
        "number,title,author,isDraft,reviewDecision,statusCheckRollup",
    ];
    let res = exec.run_retry(&gh_args).await;
    let stdout = GhExec::into_result(res, "failed to search pull requests")?;
    let prs: Vec<PullRequest> = serde_json::from_str(&stdout).map_err(|e| {
        CoworkerError::Other(anyhow::anyhow!(super::error::format_tool_error(
            super::error::ErrCode::Generic,
            &format!("failed to parse PR list: {e}"),
            "retry",
        )))
    })?;

    let mut waiting = Vec::new();
    for pr in prs {
        if pr.is_draft {
            continue;
        }
        if !pr.review_decision.eq_ignore_ascii_case("REVIEW_REQUIRED") {
            continue;
        }
        let (pass, fail, pending) = tally_checks(&pr.status_check);
        if fail > 0 || pending > 0 {
            continue;
        }
        if pass == 0 && !pr.status_check.is_empty() {
            continue;
        }
        waiting.push(pr);
        if waiting.len() >= limit as usize {
            break;
        }
    }

    if waiting.is_empty() {
        return Ok(format!("No PRs waiting for review in {repo}."));
    }
    let mut lines = vec![format!(
        "{} PR(s) waiting for review in {repo}:",
        waiting.len()
    )];
    for pr in &waiting {
        lines.push(format!(
            "#{}  {}  @{}  CI:{}  review:{}",
            pr.number,
            pr.title,
            pr.author.login,
            ci_state(&pr.status_check),
            review_state(&pr.review_decision)
        ));
    }
    Ok(lines.join("\n"))
}

// --- pr_merge_queue.go ---

pub async fn pr_list_merge_ready(exec: &GhExec, args: &Value) -> Result<String> {
    let repo = require_str(args, "repo")?;
    let limit = merge_queue_fetch_limit(args);
    let prs = fetch_open_prs_for_merge_queue(exec, &repo, limit).await?;

    let ready: Vec<_> = prs.iter().filter(|pr| is_merge_ready(pr)).collect();
    if ready.is_empty() {
        return Ok(format!(
            "No merge-ready PRs in {repo} (scanned {} open).\nNext: pr_list_merge_blocked or pr_list_waiting_review.",
            prs.len()
        )
        .trim()
        .to_string());
    }
    let mut lines = vec![format!("{} merge-ready PR(s) in {repo}:", ready.len())];
    for pr in ready {
        lines.push(format!(
            "#{}  {}  @{}  review:{}",
            pr.number,
            pr.title,
            pr.author.login,
            review_state(&pr.review_decision)
        ));
    }
    lines.push("Next: merge on GitHub or notify via notify_post_slack.".into());
    Ok(lines.join("\n"))
}

pub async fn pr_list_merge_blocked(exec: &GhExec, args: &Value) -> Result<String> {
    let repo = require_str(args, "repo")?;
    let limit = merge_queue_fetch_limit(args);
    let prs = fetch_open_prs_for_merge_queue(exec, &repo, limit).await?;

    let blocked: Vec<_> = prs
        .iter()
        .filter(|pr| is_ci_green(pr) && !is_merge_ready(pr))
        .collect();
    if blocked.is_empty() {
        return Ok(format!(
            "No CI-green-but-blocked PRs in {repo} (scanned {} open).",
            prs.len()
        ));
    }
    let mut lines = vec![format!(
        "{} PR(s) with green CI but not merge-ready in {repo}:",
        blocked.len()
    )];
    for pr in blocked {
        lines.push(format!(
            "#{}  {}  @{}  blocker:{}",
            pr.number,
            pr.title,
            pr.author.login,
            merge_queue_blocker(pr)
        ));
    }
    lines.push("Next: pr_get_merge_blockers on top rows.".into());
    Ok(lines.join("\n"))
}

// --- pr_hygiene.go ---

pub async fn pr_list_large(exec: &GhExec, args: &Value) -> Result<String> {
    let repo = require_str(args, "repo")?;
    let min_files = optional_u32(args, "min_files", 30);
    let min_lines = optional_u32(args, "min_lines", 1000);
    let mut scan_limit = optional_u32(args, "limit", 40);
    if scan_limit > 60 {
        scan_limit = 60;
    }
    let scan_limit_s = scan_limit.to_string();

    let gh_args = [
        "pr",
        "list",
        "-R",
        &repo,
        "--state",
        "open",
        "--limit",
        &scan_limit_s,
        "--json",
        "number,title,author,additions,deletions,changedFiles",
    ];
    let res = exec.run_retry(&gh_args).await;
    let stdout = GhExec::into_result(res, "failed to list open PRs")?;
    let rows: Vec<PrSizeRow> = serde_json::from_str(&stdout).map_err(|e| {
        CoworkerError::Other(anyhow::anyhow!(super::error::format_tool_error(
            super::error::ErrCode::Generic,
            &format!("failed to parse PR list: {e}"),
            "retry",
        )))
    })?;

    let large: Vec<_> = rows
        .iter()
        .filter(|pr| {
            let lines = pr.additions + pr.deletions;
            pr.changed_files >= min_files as i32 || lines >= min_lines as i32
        })
        .collect();

    let mut out = format!(
        "Large PR scan in {repo} (scanned {} open, thresholds: {min_files} files or {min_lines} lines):\n",
        rows.len()
    );
    if large.is_empty() {
        out.push_str("(none above thresholds)\n");
    } else {
        for pr in large {
            out.push_str(&format!(
                "#{}  {}  @{}  files:{}  +{}/-{}\n",
                pr.number, pr.title, pr.author.login, pr.changed_files, pr.additions, pr.deletions
            ));
        }
    }
    out.push_str("Next: pr_diff_risk_scan on top rows.");
    Ok(out.trim().to_string())
}

// --- pr_tier2.go ---

pub async fn pr_list_backport_candidates(exec: &GhExec, args: &Value) -> Result<String> {
    let repo = require_str(args, "repo")?;
    let label = optional_str(args, "label").unwrap_or_else(|| DEFAULT_BACKPORT_LABEL.to_string());
    let since_raw = optional_str(args, "since").unwrap_or_default();
    let limit = optional_u32(args, "limit", 30);
    format_merged_pr_list(exec, &repo, &since_raw, &label, limit, true).await
}

pub async fn pr_is_docs_only(exec: &GhExec, args: &Value) -> Result<String> {
    let repo = require_str(args, "repo")?;
    let pr_num = require_u32(args, "pr_number")?;
    let (count, total_add, total_del, docs_only, file_err) =
        pr_files_summary(exec, &repo, pr_num).await;

    if let Some(e) = file_err {
        return Err(e);
    }
    if count == 0 {
        return Ok(format!(
            "PR #{pr_num} in {repo}: no changed files detected."
        ));
    }
    let mut out = format!("PR #{pr_num} in {repo}: docs-only={docs_only}\n");
    out.push_str(&format!("Files: {count}  +{total_add}/-{total_del}\n"));
    if docs_only {
        out.push_str("hint: safe to deprioritize CI triage for docs-only changes");
    } else {
        out.push_str("Next: pr_get_overview or pr_diff_risk_scan");
    }
    Ok(out.trim().to_string())
}

// --- pr_review_risk.go ---

pub async fn pr_get_review_state(exec: &GhExec, args: &Value) -> Result<String> {
    let repo = require_str(args, "repo")?;
    let pr_num = require_u32(args, "pr_number")?;
    build_pr_review_state_text(exec, &repo, pr_num).await
}

pub async fn pr_diff_risk_scan(exec: &GhExec, args: &Value) -> Result<String> {
    let repo = require_str(args, "repo")?;
    let pr_num = require_u32(args, "pr_number")?;
    let files = fetch_pr_file_changes(exec, &repo, pr_num).await?;
    Ok(format_diff_risk_scan(&repo, pr_num, &files))
}

// --- pr_review_routing.go ---

pub async fn pr_get_review_routing(exec: &GhExec, args: &Value) -> Result<String> {
    let repo = require_str(args, "repo")?;
    let pr_num = require_u32(args, "pr_number")?;
    let files = list_pr_changed_paths(exec, &repo, pr_num).await?;
    let rules = load_codeowners(exec, &repo).await?;

    let mut owner_hits: std::collections::HashMap<String, HashSet<String>> =
        std::collections::HashMap::new();
    for f in &files {
        for owner in match_codeowners(&rules, f) {
            owner_hits.entry(owner).or_default().insert(f.clone());
        }
    }

    let mut out = format!(
        "Review routing for PR #{pr_num} in {repo} ({} changed files):\n",
        files.len()
    );
    if rules.is_empty() {
        out.push_str("No CODEOWNERS file found in repo root or .github/.\n");
        return Ok(out.trim().to_string());
    }
    if owner_hits.is_empty() {
        out.push_str("No CODEOWNERS patterns matched changed files.\n");
        out.push_str("Next: pr_get_review_state for requested reviewers.");
        return Ok(out.trim().to_string());
    }

    let mut rows: Vec<(String, usize)> = owner_hits
        .into_iter()
        .map(|(owner, matched)| (owner, matched.len()))
        .collect();
    rows.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    for (owner, count) in rows {
        out.push_str(&format!("{owner}  ({count} file(s))\n"));
    }
    out.push_str("Next: pr_get_review_state; mention owners in review request.");
    Ok(out.trim().to_string())
}

// --- pr_draft_comment.go ---

pub async fn pr_draft_ci_comment(exec: &GhExec, args: &Value) -> Result<String> {
    let repo = require_str(args, "repo")?;
    let pr_num = require_u32(args, "pr_number")?;
    let run_id = require_i64(args, "run_id")?;
    let analysis = analyze_run_failure(exec, &repo, run_id).await?;
    let (verdict, rule_id) = classify_failure(&analysis);
    Ok(format_draft_ci_comment(
        &repo, pr_num, &analysis, verdict, rule_id,
    ))
}

// --- shared fetch/format helpers ---

async fn fetch_pr_status(exec: &GhExec, repo: &str, pr_num: u32) -> Result<PullRequest> {
    let pr_num_s = pr_num.to_string();
    let gh_args = [
        "pr",
        "view",
        &pr_num_s,
        "-R",
        repo,
        "--json",
        "number,title,author,state,isDraft,mergeable,reviewDecision,statusCheckRollup",
    ];
    let res = exec.run_retry(&gh_args).await;
    let stdout = GhExec::into_result(res, "failed to fetch PR status")?;
    serde_json::from_str(&stdout).map_err(|e| {
        CoworkerError::Other(anyhow::anyhow!(super::error::format_tool_error(
            super::error::ErrCode::Generic,
            &format!("failed to parse PR status: {e}"),
            "retry",
        )))
    })
}

async fn fetch_pr_overview(exec: &GhExec, repo: &str, pr_num: u32) -> Result<PullRequest> {
    let pr_num_s = pr_num.to_string();
    let gh_args = [
        "pr",
        "view",
        &pr_num_s,
        "-R",
        repo,
        "--json",
        "number,title,author,state,isDraft,mergeable,reviewDecision,statusCheckRollup",
    ];
    let res = exec.run_retry(&gh_args).await;
    let stdout = GhExec::into_result(res, "failed to fetch PR overview")?;
    serde_json::from_str(&stdout).map_err(|e| {
        CoworkerError::Other(anyhow::anyhow!(super::error::format_tool_error(
            super::error::ErrCode::Generic,
            &format!("failed to parse PR overview: {e}"),
            "retry",
        )))
    })
}

async fn fetch_pr_file_changes(
    exec: &GhExec,
    repo: &str,
    pr_num: u32,
) -> Result<Vec<PrFileChange>> {
    let path = format!("repos/{repo}/pulls/{pr_num}/files");
    let gh_args = [
        "api",
        &path,
        "--paginate",
        "--jq",
        ".[] | {filename, additions, deletions, status}",
    ];
    let res = exec.run_retry(&gh_args).await;
    let stdout = GhExec::into_result(res, "failed to list changed files")?;
    let mut out = Vec::new();
    for line in stdout.lines() {
        if line.is_empty() {
            continue;
        }
        if let Ok(f) = serde_json::from_str::<PrFileChange>(line) {
            out.push(f);
        }
    }
    Ok(out)
}

async fn fetch_pr_file_patch(
    exec: &GhExec,
    repo: &str,
    pr_num: u32,
    path: &str,
) -> Result<Option<PrFilePatchRow>> {
    let api_path = format!("repos/{repo}/pulls/{pr_num}/files");
    let gh_args = [
        "api",
        &api_path,
        "--paginate",
        "--jq",
        ".[] | {filename, additions, deletions, status, patch}",
    ];
    let res = exec.run_retry(&gh_args).await;
    let stdout = GhExec::into_result(res, "failed to fetch PR file patch")?;
    for line in stdout.lines() {
        if line.is_empty() {
            continue;
        }
        if let Ok(f) = serde_json::from_str::<PrFilePatchRow>(line) {
            if f.filename == path {
                return Ok(Some(f));
            }
        }
    }
    Ok(None)
}

fn format_changed_files_list(repo: &str, pr_num: u32, files: &[PrFileChange]) -> Result<String> {
    if files.is_empty() {
        return Ok(format!("No changed files for PR #{pr_num} in {repo}."));
    }
    let mut total_add = 0i32;
    let mut total_del = 0i32;
    let mut lines = vec![format!(
        "{} changed file(s) in {repo}#{pr_num}:",
        files.len()
    )];
    for f in files {
        total_add += f.additions;
        total_del += f.deletions;
        lines.push(format!(
            "{}  +{}/-{}  ({})",
            f.filename, f.additions, f.deletions, f.status
        ));
    }
    lines.push(format!("totals: +{total_add}/-{total_del}"));
    lines
        .push("Next: pr_get_diff with path=<filename> for one file at a time on large PRs.".into());
    Ok(lines.join("\n"))
}

async fn build_proverview_text(
    exec: &GhExec,
    repo: &str,
    pr_num: u32,
    pr: &PullRequest,
) -> Result<String> {
    let (pass, fail, pending) = tally_checks(&pr.status_check);
    let (file_count, total_add, total_del, docs_only, file_err) =
        pr_files_summary(exec, repo, pr_num).await;

    let mut out = format!("PR #{} {}\n", pr.number, pr.title);
    out.push_str(&format!(
        "Author: @{}   State: {}",
        pr.author.login,
        pr.state.to_ascii_lowercase()
    ));
    if pr.is_draft {
        out.push_str(" (draft)");
    }
    out.push('\n');
    out.push_str(&format!(
        "CI: {pass} passing / {fail} failing / {pending} pending\n"
    ));
    let ext = format_external_check_summary(&pr.status_check);
    if !ext.is_empty() {
        out.push_str(&ext);
    }
    out.push_str(&format!("Review: {}\n", review_state(&pr.review_decision)));
    out.push_str(&format!(
        "Mergeable: {}\n",
        mergeable_state(&pr.mergeable, fail, pending)
    ));

    if let Some(e) = file_err {
        out.push_str(&format!("Files: (unavailable — {e})\n"));
    } else {
        out.push_str(&format!(
            "Files: {file_count} changed  +{total_add}/-{total_del}"
        ));
        if docs_only && file_count > 0 {
            out.push_str("  (docs-only)");
        }
        out.push('\n');
    }

    match failing_runs_for_pr(exec, repo, pr_num).await {
        Err(e) => {
            out.push_str(&format!("\nFailing CI runs: (could not list — {e})"));
        }
        Ok((head_sha, failed)) => {
            if failed.is_empty() {
                out.push_str(&format!(
                    "\nFailing CI runs: none on GitHub Actions @{}",
                    short_sha(&head_sha)
                ));
                if fail > 0 {
                    out.push_str(" (external CI may still be failing)");
                }
            } else {
                out.push_str(&format!(
                    "\n{} failing run(s) for PR #{pr_num} @{}:\n",
                    failed.len(),
                    short_sha(&head_sha)
                ));
                for r in failed {
                    out.push_str(&format!(
                        "{}  {}  {}\n",
                        r.database_id,
                        r.workflow_name,
                        r.conclusion.to_ascii_lowercase()
                    ));
                }
            }
        }
    }

    Ok(out.trim().to_string())
}

async fn build_pr_merge_blockers_text(exec: &GhExec, repo: &str, pr_num: u32) -> Result<String> {
    let pr_num_s = pr_num.to_string();
    let gh_args = [
        "pr",
        "view",
        &pr_num_s,
        "-R",
        repo,
        "--json",
        "number,title,author,isDraft,mergeable,reviewDecision,statusCheckRollup",
    ];
    let res = exec.run_retry(&gh_args).await;
    let stdout = GhExec::into_result(res, "failed to fetch PR blockers")?;
    let pr: PullRequest = serde_json::from_str(&stdout).map_err(|e| {
        CoworkerError::Other(anyhow::anyhow!(super::error::format_tool_error(
            super::error::ErrCode::Generic,
            &format!("failed to parse PR blockers: {e}"),
            "retry",
        )))
    })?;

    let (pass, fail, pending) = tally_checks(&pr.status_check);
    let blockers = merge_blockers(&pr, fail, pending);

    let mut out = format!("PR #{} {}\n", pr.number, pr.title);
    out.push_str(&format!("Author: @{}", pr.author.login));
    if pr.is_draft {
        out.push_str("  (draft)");
    }
    out.push('\n');
    out.push_str(&format!(
        "CI: {pass} passing / {fail} failing / {pending} pending\n"
    ));
    out.push_str(&format!("Review: {}\n", review_state(&pr.review_decision)));
    out.push_str(&format!(
        "Mergeable: {}\n",
        mergeable_state(&pr.mergeable, fail, pending)
    ));
    if blockers.is_empty() {
        out.push_str("\nBlockers: (none)");
    } else {
        out.push_str("\nBlockers:\n");
        for bl in blockers {
            out.push_str(&format!("- {bl}\n"));
        }
    }
    Ok(out.trim().to_string())
}

async fn build_pr_review_state_text(exec: &GhExec, repo: &str, pr_num: u32) -> Result<String> {
    let pr_num_s = pr_num.to_string();
    let gh_args = [
        "pr",
        "view",
        &pr_num_s,
        "-R",
        repo,
        "--json",
        "number,title,reviewDecision,reviewRequests,latestReviews",
    ];
    let res = exec.run_retry(&gh_args).await;
    let stdout = GhExec::into_result(res, "failed to fetch PR review state")?;
    let pr: PrReviewView = serde_json::from_str(&stdout).map_err(|e| {
        CoworkerError::Other(anyhow::anyhow!(super::error::format_tool_error(
            super::error::ErrCode::Generic,
            &format!("failed to parse review state: {e}"),
            "retry",
        )))
    })?;

    let mut out = format!("PR #{} {}\n", pr.number, pr.title);
    out.push_str(&format!(
        "Review decision: {}\n",
        review_state(&pr.review_decision)
    ));

    if !pr.review_requests.is_empty() {
        out.push_str("Requested reviewers:");
        for rr in &pr.review_requests {
            if !rr.login.is_empty() {
                out.push_str(&format!(" @{}", rr.login));
            }
        }
        out.push('\n');
    }

    if !pr.latest_reviews.is_empty() {
        out.push_str("Latest reviews:\n");
        for lr in &pr.latest_reviews {
            let state = if lr.state.trim().is_empty() {
                "COMMENTED".to_string()
            } else {
                lr.state.trim().to_ascii_uppercase()
            };
            let snippet = clip_for_log(lr.body.trim(), 80);
            if !snippet.is_empty() {
                out.push_str(&format!("- @{} {state}: {snippet:?}\n", lr.author.login));
            } else {
                out.push_str(&format!("- @{} {state}\n", lr.author.login));
            }
        }
    }

    let path = format!("repos/{repo}/pulls/{pr_num}/comments");
    let c_args = [
        "api",
        &path,
        "--paginate",
        "--jq",
        ".[] | {path, line, user: .user.login, body}",
    ];
    let c_res = exec.run_retry(&c_args).await;
    if c_res.err.is_none() && !c_res.stdout.trim().is_empty() {
        let mut lines: Vec<&str> = c_res.stdout.lines().filter(|l| !l.is_empty()).collect();
        if lines.len() > MAX_REVIEW_COMMENTS {
            lines.truncate(MAX_REVIEW_COMMENTS);
            out.push_str(&format!("Inline comments (first {MAX_REVIEW_COMMENTS}):\n"));
        } else {
            out.push_str("Inline comments:\n");
        }
        for line in lines {
            if let Ok(c) = serde_json::from_str::<InlineComment>(line) {
                let snippet = clip_for_log(c.body.trim(), 100);
                if c.line > 0 {
                    out.push_str(&format!(
                        "- {}:{} @{}: {snippet:?}\n",
                        c.path, c.line, c.user.login
                    ));
                } else {
                    out.push_str(&format!("- {} @{}: {snippet:?}\n", c.path, c.user.login));
                }
            }
        }
    }

    if pr.review_requests.is_empty() && pr.latest_reviews.is_empty() {
        out.push_str("No pending review requests or reviews recorded.");
    }
    out.push_str("\nNext: pr_get_merge_blockers or pr_post_comment for follow-up.");
    Ok(out.trim().to_string())
}

async fn format_merged_pr_list(
    exec: &GhExec,
    repo: &str,
    since_raw: &str,
    label: &str,
    limit: u32,
    backport_hint: bool,
) -> Result<String> {
    let since_date = merged_since_date(since_raw)?;
    let limit_s = limit.to_string();
    let search = format!("merged:>={since_date}");

    let mut gh_args = vec![
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
    let label_owned;
    if !label.trim().is_empty() {
        label_owned = label.trim().to_string();
        gh_args.push("--label");
        gh_args.push(&label_owned);
    }

    let res = exec.run_retry(&gh_args).await;
    let stdout = GhExec::into_result(res, "failed to list merged PRs")?;
    let prs: Vec<PrMergedRow> = serde_json::from_str(&stdout).map_err(|e| {
        CoworkerError::Other(anyhow::anyhow!(super::error::format_tool_error(
            super::error::ErrCode::Generic,
            &format!("failed to parse merged PR list: {e}"),
            "retry",
        )))
    })?;

    if prs.is_empty() {
        return Ok(if label.trim().is_empty() {
            format!("No merged PRs in {repo} since {since_date}.")
        } else {
            format!("No merged PRs in {repo} since {since_date} with label {label:?}.")
        });
    }

    let mut out = if label.trim().is_empty() {
        format!("{} merged PR(s) in {repo} since {since_date}:\n", prs.len())
    } else {
        format!(
            "{} merged PR(s) in {repo} since {since_date} (label {label:?}):\n",
            prs.len()
        )
    };
    for pr in &prs {
        let merged = pr.merged_at.get(..10).unwrap_or(&pr.merged_at);
        out.push_str(&format!(
            "#{}  {}  @{}  merged:{merged}\n",
            pr.number, pr.title, pr.author.login
        ));
    }
    if backport_hint && label == DEFAULT_BACKPORT_LABEL {
        out.push_str("Next: pr_create_backport for each row.");
    }
    Ok(out.trim().to_string())
}

fn merged_since_date(since: &str) -> Result<String> {
    if since.is_empty() {
        let d = Utc::now().date_naive() - chrono::Days::new(14);
        return Ok(d.format("%Y-%m-%d").to_string());
    }
    if since.len() == 10 && since.as_bytes().get(4) == Some(&b'-') {
        return Ok(since.to_string());
    }
    let d: i64 = since.parse().map_err(|_| {
        CoworkerError::Other(anyhow::anyhow!(super::error::format_tool_error(
            super::error::ErrCode::Validation,
            &format!("since must be YYYY-MM-DD or days as integer, got {since:?}"),
            "Use YYYY-MM-DD or a number of days",
        )))
    })?;
    let days = if d <= 0 { 14 } else { d as u32 };
    let date = Utc::now().date_naive() - chrono::Days::new(days as u64);
    Ok(date.format("%Y-%m-%d").to_string())
}

async fn pr_files_summary(
    exec: &GhExec,
    repo: &str,
    pr_num: u32,
) -> (i32, i32, i32, bool, Option<CoworkerError>) {
    let path = format!("repos/{repo}/pulls/{pr_num}/files");
    let gh_args = [
        "api",
        &path,
        "--paginate",
        "--jq",
        ".[] | {filename, additions, deletions}",
    ];
    let res = exec.run_retry(&gh_args).await;
    if res.err.is_some() {
        return (
            0,
            0,
            0,
            false,
            Some(res.wrap("failed to list changed files")),
        );
    }
    let trimmed = res.stdout.trim();
    if trimmed.is_empty() {
        return (0, 0, 0, false, None);
    }
    let mut count = 0i32;
    let mut total_add = 0i32;
    let mut total_del = 0i32;
    let mut docs_only = true;
    for line in trimmed.lines() {
        if line.is_empty() {
            continue;
        }
        #[derive(Deserialize)]
        struct Row {
            filename: String,
            additions: i32,
            deletions: i32,
        }
        let Ok(f) = serde_json::from_str::<Row>(line) else {
            continue;
        };
        count += 1;
        total_add += f.additions;
        total_del += f.deletions;
        if !is_docs_path(&f.filename) {
            docs_only = false;
        }
    }
    (count, total_add, total_del, docs_only, None)
}

fn is_docs_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".md") || lower.ends_with(".rst") {
        return true;
    }
    lower.starts_with("docs/")
        || lower.starts_with("doc/")
        || lower.contains("/docs/")
        || lower == "readme.md"
        || lower == "changelog.md"
}

fn merge_queue_fetch_limit(args: &Value) -> u32 {
    let mut limit = optional_u32(args, "limit", 30);
    if limit > 50 {
        limit = 50;
    }
    limit
}

async fn fetch_open_prs_for_merge_queue(
    exec: &GhExec,
    repo: &str,
    limit: u32,
) -> Result<Vec<PullRequest>> {
    let limit_s = limit.to_string();
    let gh_args = [
        "pr",
        "list",
        "-R",
        repo,
        "--state",
        "open",
        "--limit",
        &limit_s,
        "--json",
        "number,title,author,isDraft,mergeable,reviewDecision,statusCheckRollup",
    ];
    let res = exec.run_retry(&gh_args).await;
    let stdout = GhExec::into_result(res, "failed to list open PRs")?;
    serde_json::from_str(&stdout).map_err(|e| {
        CoworkerError::Other(anyhow::anyhow!(super::error::format_tool_error(
            super::error::ErrCode::Generic,
            &format!("failed to parse PR list: {e}"),
            "retry",
        )))
    })
}

fn is_ci_green(pr: &PullRequest) -> bool {
    let (pass, fail, pending) = tally_checks(&pr.status_check);
    if fail > 0 || pending > 0 {
        return false;
    }
    pass != 0 || pr.status_check.is_empty()
}

fn is_merge_ready(pr: &PullRequest) -> bool {
    if pr.is_draft {
        return false;
    }
    if !is_ci_green(pr) {
        return false;
    }
    if !pr.review_decision.trim().eq_ignore_ascii_case("APPROVED") {
        return false;
    }
    pr.mergeable.trim().eq_ignore_ascii_case("MERGEABLE")
}

fn merge_queue_blocker(pr: &PullRequest) -> &'static str {
    if pr.is_draft {
        return "draft";
    }
    match pr.mergeable.trim().to_ascii_uppercase().as_str() {
        "CONFLICTING" => return "merge conflicts",
        "UNKNOWN" | "" => return "mergeability unknown",
        _ => {}
    }
    match pr.review_decision.trim().to_ascii_uppercase().as_str() {
        "REVIEW_REQUIRED" => return "review required",
        "CHANGES_REQUESTED" => return "changes requested",
        _ => {}
    }
    let (_, fail, pending) = tally_checks(&pr.status_check);
    if fail > 0 {
        return "CI failing";
    }
    if pending > 0 {
        return "CI pending";
    }
    "other blocker (branch protection?)"
}

fn format_diff_risk_scan(repo: &str, pr_num: u32, files: &[PrFileChange]) -> String {
    if files.is_empty() {
        return format!("No changed files for {repo}#{pr_num}.");
    }

    let mut flags = HashSet::new();
    let mut flagged_files = Vec::new();
    let mut total_add = 0i32;
    let mut total_del = 0i32;

    for f in files {
        total_add += f.additions;
        total_del += f.deletions;
        let base = Path::new(&f.filename)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let dir = Path::new(&f.filename)
            .parent()
            .and_then(|p| p.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();

        if is_lockfile(&base) {
            flags.insert("lockfile");
            flagged_files.push(f.filename.clone());
        }
        if dir.contains("migration") || f.filename.contains("/migrate/") {
            flags.insert("migration");
            flagged_files.push(f.filename.clone());
        }
        if f.filename.starts_with(".github/workflows/") {
            flags.insert("workflow_changed");
            flagged_files.push(f.filename.clone());
        }
        if f.additions + f.deletions > 500 {
            flags.insert("large_diff");
        }
        if f.status == "removed"
            && (base.contains("_test.") || dir.contains(&format!("{MAIN_SEPARATOR}test")))
        {
            flags.insert("tests_removed");
            flagged_files.push(f.filename.clone());
        }
    }

    let mut out = format!(
        "Risk scan {repo}#{pr_num} ({} files, +{total_add}/-{total_del}):\n",
        files.len()
    );
    if flags.is_empty() {
        out.push_str("RISK flags: (none significant)\n");
    } else {
        out.push_str("RISK flags:");
        for name in &flags {
            out.push_str(&format!(" {name}"));
        }
        out.push('\n');
    }
    if !flagged_files.is_empty() {
        out.push_str("Notable files:\n");
        let mut seen = HashSet::new();
        let mut n = 0;
        for p in flagged_files {
            if !seen.insert(p.clone()) {
                continue;
            }
            out.push_str(&format!("- {p}\n"));
            n += 1;
            if n >= 12 {
                break;
            }
        }
    }
    out.push_str("Next: pr_get_diff for code review; pr_get_overview for CI context.");
    out.trim().to_string()
}

fn is_lockfile(base: &str) -> bool {
    matches!(
        base,
        "go.sum"
            | "package-lock.json"
            | "yarn.lock"
            | "pnpm-lock.yaml"
            | "cargo.lock"
            | "gemfile.lock"
            | "poetry.lock"
    ) || base.ends_with(".lock")
}

async fn list_pr_changed_paths(exec: &GhExec, repo: &str, pr_num: u32) -> Result<Vec<String>> {
    let pr_num_s = pr_num.to_string();
    let gh_args = ["pr", "view", &pr_num_s, "-R", repo, "--json", "files"];
    let res = exec.run_retry(&gh_args).await;
    let stdout = GhExec::into_result(res, &format!("failed to load PR #{pr_num} files"))?;
    #[derive(Deserialize)]
    struct Payload {
        files: Vec<FileRow>,
    }
    #[derive(Deserialize)]
    struct FileRow {
        path: String,
    }
    let payload: Payload = serde_json::from_str(&stdout).map_err(|e| {
        CoworkerError::Other(anyhow::anyhow!(super::error::format_tool_error(
            super::error::ErrCode::Generic,
            &format!("parse PR files: {e}"),
            "retry",
        )))
    })?;
    Ok(payload
        .files
        .into_iter()
        .filter_map(|f| {
            let p = f.path.trim();
            if p.is_empty() {
                None
            } else {
                Some(p.to_string())
            }
        })
        .collect())
}

async fn load_codeowners(exec: &GhExec, repo: &str) -> Result<Vec<CodeownersRule>> {
    for loc in [".github/CODEOWNERS", "CODEOWNERS"] {
        let rules = fetch_codeowners_at(exec, repo, loc).await?;
        if !rules.is_empty() {
            return Ok(rules);
        }
    }
    Ok(Vec::new())
}

async fn fetch_codeowners_at(
    exec: &GhExec,
    repo: &str,
    file_path: &str,
) -> Result<Vec<CodeownersRule>> {
    let path = format!("repos/{repo}/contents/{file_path}");
    let gh_args = ["api", &path];
    let res = exec.run_retry(&gh_args).await;
    if res.err.is_some() {
        let low = res.combined().to_ascii_lowercase();
        if low.contains("http 404") || low.contains("not found") {
            return Ok(Vec::new());
        }
        return Err(res.wrap("fetch CODEOWNERS"));
    }
    #[derive(Deserialize)]
    struct Payload {
        content: String,
        encoding: String,
    }
    let payload: Payload = serde_json::from_str(&res.stdout).map_err(|e| {
        CoworkerError::Other(anyhow::anyhow!(super::error::format_tool_error(
            super::error::ErrCode::Generic,
            &format!("parse CODEOWNERS metadata: {e}"),
            "retry",
        )))
    })?;
    if payload.encoding != "base64" {
        return Err(CoworkerError::Other(anyhow::anyhow!(
            super::error::format_tool_error(
                super::error::ErrCode::Generic,
                &format!("unexpected CODEOWNERS encoding: {}", payload.encoding),
                "retry",
            )
        )));
    }
    let cleaned = payload.content.replace('\n', "");
    let raw = base64_decode(&cleaned).map_err(|e| {
        CoworkerError::Other(anyhow::anyhow!(super::error::format_tool_error(
            super::error::ErrCode::Generic,
            &format!("decode CODEOWNERS: {e}"),
            "retry",
        )))
    })?;
    Ok(parse_codeowners(&raw))
}

fn base64_decode(input: &str) -> std::result::Result<String, String> {
    const TABLE: &[u8; 256] = &{
        let mut t = [255u8; 256];
        let mut i = 0u8;
        while i < 64 {
            let c = match i {
                0..=25 => b'A' + i,
                26..=51 => b'a' + (i - 26),
                52..=61 => b'0' + (i - 52),
                62 => b'+',
                _ => b'/',
            };
            t[c as usize] = i;
            i += 1;
        }
        t
    };
    let bytes: Vec<u8> = input.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    let mut buf = 0u32;
    let mut bits = 0u32;
    for &b in &bytes {
        if b == b'=' {
            break;
        }
        let val = TABLE[b as usize];
        if val == 255 {
            return Err(format!("invalid base64 byte {b}"));
        }
        buf = (buf << 6) | val as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
            buf &= (1 << bits) - 1;
        }
    }
    String::from_utf8(out).map_err(|e| e.to_string())
}

fn parse_codeowners(text: &str) -> Vec<CodeownersRule> {
    let mut rules = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 2 {
            continue;
        }
        rules.push(CodeownersRule {
            pattern: parts[0].to_string(),
            owners: parts[1..].iter().map(|s| s.to_string()).collect(),
        });
    }
    rules
}

fn match_codeowners(rules: &[CodeownersRule], file_path: &str) -> Vec<String> {
    let file_path = file_path.trim_start_matches("./");
    let mut owners = Vec::new();
    let mut seen = HashSet::new();
    for rule in rules {
        if !codeowners_pattern_match(&rule.pattern, file_path) {
            continue;
        }
        for o in &rule.owners {
            if seen.insert(o.clone()) {
                owners.push(o.clone());
            }
        }
    }
    owners
}

fn codeowners_pattern_match(pattern: &str, file_path: &str) -> bool {
    let pattern = pattern.trim();
    if pattern.is_empty() {
        return false;
    }
    if matches!(pattern, "*" | "**" | "/**") {
        return true;
    }
    let file_path = file_path.trim_start_matches("./").trim_start_matches('/');
    if pattern.contains("**") {
        let mut prefix = pattern.trim_end_matches("**");
        prefix = prefix.trim_end_matches('/');
        if prefix.is_empty() || prefix == "/" {
            return true;
        }
        let prefix = prefix.trim_start_matches('/');
        return file_path.starts_with(&format!("{prefix}/")) || file_path == prefix;
    }
    let pattern = if let Some(p) = pattern.strip_prefix('/') {
        p
    } else if !pattern.contains('/') {
        let base = Path::new(file_path)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(file_path);
        return glob_match(pattern, base);
    } else {
        pattern
    };
    if let Some(dir) = pattern.strip_suffix("/*") {
        let dir = dir.trim_start_matches('/');
        return file_path == dir || file_path.starts_with(&format!("{dir}/"));
    }
    if pattern.ends_with('*') && !pattern.trim_end_matches('*').contains('/') {
        let suffix = pattern.trim_start_matches('*');
        return file_path.ends_with(suffix);
    }
    glob_match(pattern, file_path)
}

fn glob_match(pattern: &str, text: &str) -> bool {
    // Simple glob: * matches any sequence, ? matches one char
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    fn rec(p: &[char], t: &[char], pi: usize, ti: usize) -> bool {
        if pi == p.len() {
            return ti == t.len();
        }
        if p[pi] == '*' {
            if pi + 1 == p.len() {
                return true;
            }
            for i in ti..=t.len() {
                if rec(p, t, pi + 1, i) {
                    return true;
                }
            }
            return false;
        }
        if ti == t.len() {
            return false;
        }
        if p[pi] == '?' || p[pi] == t[ti] {
            return rec(p, t, pi + 1, ti + 1);
        }
        false
    }
    rec(&p, &t, 0, 0)
}

// --- exported formatting helpers for pr_batch ---

pub(crate) fn format_pr_list_line(pr: &PullRequest) -> String {
    let draft = if pr.is_draft { " [draft]" } else { "" };
    format!(
        "#{}  {}  @{}  CI:{}  review:{}{}",
        pr.number,
        pr.title,
        pr.author.login,
        ci_state(&pr.status_check),
        review_state(&pr.review_decision),
        draft
    )
}

pub(crate) fn format_proverview_batch_line(pr: &PullRequestOverviewBatch) -> String {
    let (pass, fail, pending) = tally_checks(&pr.status_check);
    let draft = if pr.is_draft { " draft" } else { "" };
    format!(
        "#{}  {}  @{}  CI:{pass}/{fail}/{pending}  review:{}  files:{} +{}/-{}{}",
        pr.number,
        pr.title,
        pr.author.login,
        review_state(&pr.review_decision),
        pr.changed_files,
        pr.additions,
        pr.deletions,
        draft
    )
}

fn format_pr_status(pr: &PullRequest) -> String {
    let (pass, fail, pending) = tally_checks(&pr.status_check);
    let mut out = format!("PR #{} {}\n", pr.number, pr.title);
    out.push_str(&format!(
        "Author: @{}   State: {}",
        pr.author.login,
        pr.state.to_ascii_lowercase()
    ));
    if pr.is_draft {
        out.push_str(" (draft)");
    }
    out.push('\n');
    out.push_str(&format!(
        "CI: {pass} passing / {fail} failing / {pending} pending\n"
    ));
    let ext = format_external_check_summary(&pr.status_check);
    if !ext.is_empty() {
        out.push_str(&ext);
    }
    out.push_str(&format!("Review: {}\n", review_state(&pr.review_decision)));
    out.push_str(&format!(
        "Mergeable: {}",
        mergeable_state(&pr.mergeable, fail, pending)
    ));
    out.trim().to_string()
}

fn merge_blockers(pr: &PullRequest, fail: i32, pending: i32) -> Vec<String> {
    let mut blockers = Vec::new();
    if pr.is_draft {
        blockers.push("draft PR".into());
    }
    if pr.mergeable.eq_ignore_ascii_case("CONFLICTING") {
        blockers.push("merge conflicts".into());
    }
    for c in &pr.status_check {
        let name = check_display_name(c);
        let name = if name.is_empty() {
            "check".to_string()
        } else {
            name
        };
        let verdict = check_verdict(c);
        match verdict.to_ascii_uppercase().as_str() {
            "FAILURE" | "ERROR" | "TIMED_OUT" | "CANCELLED" | "STARTUP_FAILURE"
            | "ACTION_REQUIRED" => {
                blockers.push(format!(
                    "CI failing: {} ({})",
                    name,
                    verdict.to_ascii_lowercase()
                ));
            }
            "PENDING" | "QUEUED" | "IN_PROGRESS" | "EXPECTED" | ""
                if !matches!(
                    verdict.to_ascii_uppercase().as_str(),
                    "SUCCESS" | "NEUTRAL" | "SKIPPED"
                ) =>
            {
                blockers.push(format!("CI pending: {name}"));
            }
            _ => {}
        }
    }
    if fail > 0 && !has_prefix_blocker(&blockers, "CI failing") {
        blockers.push(format!("CI failing ({fail} check(s))"));
    }
    if pending > 0 && !has_prefix_blocker(&blockers, "CI pending") {
        blockers.push(format!("CI pending ({pending} check(s))"));
    }
    match pr.review_decision.to_ascii_uppercase().as_str() {
        "REVIEW_REQUIRED" => blockers.push("review required".into()),
        "CHANGES_REQUESTED" => blockers.push("changes requested".into()),
        _ => {}
    }
    blockers
}

fn has_prefix_blocker(blockers: &[String], prefix: &str) -> bool {
    blockers.iter().any(|b| b.starts_with(prefix))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::checks::CheckRollup;

    fn pr_with(mergeable: &str, review: &str, checks: Vec<CheckRollup>) -> PullRequest {
        PullRequest {
            number: 1,
            title: "t".into(),
            author: PrAuthor::default(),
            state: "OPEN".into(),
            is_draft: false,
            mergeable: mergeable.into(),
            review_decision: review.into(),
            status_check: checks,
        }
    }

    fn check_run(conclusion: &str) -> CheckRollup {
        CheckRollup {
            typename: Some("CheckRun".into()),
            name: Some("ci".into()),
            context: None,
            status: Some("COMPLETED".into()),
            conclusion: Some(conclusion.into()),
            state: None,
            details_url: None,
            target_url: None,
        }
    }

    #[test]
    fn docs_path_recognizes_common_doc_locations() {
        assert!(is_docs_path("README.md"));
        assert!(is_docs_path("docs/intro.md"));
        assert!(is_docs_path("doc/guide.rst"));
        assert!(is_docs_path("src/docs/foo.md"));
        assert!(is_docs_path("CHANGELOG.md"));
        assert!(!is_docs_path("src/main.rs"));
        assert!(!is_docs_path("docs.go"));
    }

    #[test]
    fn lockfile_detection_covers_common_names() {
        assert!(is_lockfile("package-lock.json"));
        assert!(is_lockfile("yarn.lock"));
        assert!(is_lockfile("Cargo.lock"));
        assert!(is_lockfile("poetry.lock"));
        assert!(is_lockfile("go.sum"));
        assert!(is_lockfile("foo.lock"));
        assert!(!is_lockfile("Cargo.toml"));
        assert!(!is_lockfile("lockfile.txt"));
    }

    #[test]
    fn ci_green_when_all_pass_or_empty() {
        assert!(is_ci_green(&pr_with(
            "MERGEABLE",
            "APPROVED",
            vec![check_run("SUCCESS")]
        )));
        assert!(is_ci_green(&pr_with("MERGEABLE", "APPROVED", vec![])));
        assert!(!is_ci_green(&pr_with(
            "MERGEABLE",
            "APPROVED",
            vec![check_run("FAILURE")]
        )));
        assert!(!is_ci_green(&pr_with(
            "MERGEABLE",
            "APPROVED",
            vec![check_run("EXPECTED")]
        )));
    }

    #[test]
    fn merge_ready_requires_all_conditions() {
        let green = vec![check_run("SUCCESS")];
        assert!(is_merge_ready(&pr_with(
            "MERGEABLE",
            "APPROVED",
            green.clone()
        )));
        // draft
        let mut draft = pr_with("MERGEABLE", "APPROVED", green.clone());
        draft.is_draft = true;
        assert!(!is_merge_ready(&draft));
        // ci failing
        assert!(!is_merge_ready(&pr_with(
            "MERGEABLE",
            "APPROVED",
            vec![check_run("FAILURE")]
        )));
        // review required
        assert!(!is_merge_ready(&pr_with(
            "MERGEABLE",
            "REVIEW_REQUIRED",
            green.clone()
        )));
        // not mergeable
        assert!(!is_merge_ready(&pr_with(
            "CONFLICTING",
            "APPROVED",
            green.clone()
        )));
    }

    #[test]
    fn merge_queue_blocker_reports_specific_reason() {
        let mut draft = pr_with("MERGEABLE", "APPROVED", vec![]);
        draft.is_draft = true;
        assert_eq!(merge_queue_blocker(&draft), "draft");
        assert_eq!(
            merge_queue_blocker(&pr_with("CONFLICTING", "APPROVED", vec![])),
            "merge conflicts"
        );
        assert_eq!(
            merge_queue_blocker(&pr_with("UNKNOWN", "APPROVED", vec![])),
            "mergeability unknown"
        );
        assert_eq!(
            merge_queue_blocker(&pr_with("MERGEABLE", "REVIEW_REQUIRED", vec![])),
            "review required"
        );
        assert_eq!(
            merge_queue_blocker(&pr_with("MERGEABLE", "CHANGES_REQUESTED", vec![])),
            "changes requested"
        );
        assert_eq!(
            merge_queue_blocker(&pr_with(
                "MERGEABLE",
                "APPROVED",
                vec![check_run("FAILURE")]
            )),
            "CI failing"
        );
        assert_eq!(
            merge_queue_blocker(&pr_with(
                "MERGEABLE",
                "APPROVED",
                vec![check_run("EXPECTED")]
            )),
            "CI pending"
        );
        // All green but still no approval -> falls through to "other blocker" only if review approved; here APPROVED + green -> "other blocker" by elimination
        assert_eq!(
            merge_queue_blocker(&pr_with(
                "MERGEABLE",
                "APPROVED",
                vec![check_run("SUCCESS")]
            )),
            "other blocker (branch protection?)"
        );
    }

    #[test]
    fn merge_blockers_combines_multiple_signals() {
        let pr = pr_with(
            "CONFLICTING",
            "REVIEW_REQUIRED",
            vec![check_run("FAILURE"), check_run("SUCCESS")],
        );
        let b = merge_blockers(&pr, 1, 0);
        assert!(b.iter().any(|x| x.contains("merge conflicts")));
        assert!(b.iter().any(|x| x.contains("CI failing")));
        assert!(b.iter().any(|x| x == "review required"));
    }

    #[test]
    fn merge_blockers_does_not_double_count_prefix() {
        // Two failing checks -> still only one "CI failing:" entry plus a tally fallback if no name match.
        let pr = pr_with(
            "MERGEABLE",
            "APPROVED",
            vec![check_run("FAILURE"), check_run("ERROR")],
        );
        let b = merge_blockers(&pr, 2, 0);
        let ci_failing_named = b.iter().filter(|x| x.starts_with("CI failing:")).count();
        assert_eq!(ci_failing_named, 2);
        // tally fallback should NOT trigger because prefix blocker exists
        assert!(!b.iter().any(|x| x == "CI failing (2 check(s))"));
    }

    #[test]
    fn has_prefix_blocker_basic() {
        let bs = vec!["CI failing: build".to_string(), "draft PR".to_string()];
        assert!(has_prefix_blocker(&bs, "CI failing"));
        assert!(!has_prefix_blocker(&bs, "CI pending"));
    }

    #[test]
    fn parse_codeowners_skips_blanks_and_comments() {
        let text = "# comment\n\n*       @team-a\n/src/   @bob @alice\nbadline\n";
        let rules = parse_codeowners(text);
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].pattern, "*");
        assert_eq!(rules[0].owners, vec!["@team-a"]);
        assert_eq!(rules[1].pattern, "/src/");
        assert_eq!(rules[1].owners, vec!["@bob", "@alice"]);
    }

    #[test]
    fn match_codeowners_strips_dot_slash_and_dedups() {
        let rules = vec![
            CodeownersRule {
                pattern: "*".into(),
                owners: vec!["@team-a".into()],
            },
            CodeownersRule {
                pattern: "/src/**".into(),
                owners: vec!["@bob".into(), "@team-a".into()],
            },
        ];
        let owners = match_codeowners(&rules, "./src/main.rs");
        assert_eq!(owners, vec!["@team-a", "@bob"]);
        assert!(match_codeowners(&rules, "./nonexistent").contains(&"@team-a".to_string()));
    }

    #[test]
    fn codeowners_pattern_match_variants() {
        assert!(codeowners_pattern_match("*", "anything.rs"));
        assert!(codeowners_pattern_match("**", "any/path.rs"));
        assert!(codeowners_pattern_match("/**", "any/path.rs"));
        // ** prefix
        assert!(codeowners_pattern_match("/src/**", "src/main.rs"));
        assert!(!codeowners_pattern_match("/src/**", "tests/foo.rs"));
        // directory match
        assert!(codeowners_pattern_match("/docs/*", "docs/intro.md"));
        // basename match when no slash in pattern
        assert!(codeowners_pattern_match("*.ts", "src/foo.ts"));
        assert!(!codeowners_pattern_match("*.ts", "src/foo.rs"));
        // anchored pattern
        assert!(codeowners_pattern_match("/Cargo.toml", "Cargo.toml"));
    }

    #[test]
    fn glob_match_wildcards() {
        assert!(glob_match("*.rs", "main.rs"));
        assert!(glob_match("foo*.rs", "foobar.rs"));
        assert!(glob_match("?atch", "catch"));
        assert!(!glob_match("?atch", "cat"));
        assert!(glob_match("exact", "exact"));
        assert!(!glob_match("exact", "other"));
        assert!(glob_match("*", ""));
    }

    #[test]
    fn format_pr_status_renders_fields() {
        let pr = pr_with("MERGEABLE", "APPROVED", vec![check_run("SUCCESS")]);
        let s = format_pr_status(&pr);
        assert!(s.contains("PR #1 t"));
        assert!(s.contains("CI: 1 passing / 0 failing / 0 pending"));
        assert!(s.contains("Review: approved"));
        assert!(s.contains("Mergeable: yes"));
    }

    #[test]
    fn format_pr_status_marks_draft() {
        let mut pr = pr_with("MERGEABLE", "APPROVED", vec![]);
        pr.is_draft = true;
        let s = format_pr_status(&pr);
        assert!(s.contains("(draft)"));
    }

    #[test]
    fn format_pr_list_line_includes_draft_suffix() {
        let mut pr = pr_with("MERGEABLE", "APPROVED", vec![check_run("SUCCESS")]);
        let line = format_pr_list_line(&pr);
        assert!(!line.contains("[draft]"));
        pr.is_draft = true;
        let line = format_pr_list_line(&pr);
        assert!(line.contains("[draft]"));
    }

    #[test]
    fn format_proverview_batch_line_renders_counts() {
        let pr = PullRequestOverviewBatch {
            number: 42,
            title: "feat".into(),
            author: PrAuthor { login: "u".into() },
            is_draft: false,
            review_decision: "APPROVED".into(),
            status_check: vec![check_run("SUCCESS"), check_run("FAILURE")],
            additions: 100,
            deletions: 5,
            changed_files: 3,
        };
        let line = format_proverview_batch_line(&pr);
        assert!(line.contains("#42  feat  @u"));
        assert!(line.contains("CI:1/1/0"));
        assert!(line.contains("review:approved"));
        assert!(line.contains("files:3 +100/-5"));
    }

    #[test]
    fn format_diff_risk_scan_empty_files() {
        let s = format_diff_risk_scan("repo", 9, &[]);
        assert!(s.contains("No changed files for repo#9"));
    }

    #[test]
    fn format_diff_risk_scan_flags_lockfile_and_large_diff() {
        let files = vec![
            PrFileChange {
                filename: "package-lock.json".into(),
                additions: 10,
                deletions: 0,
                status: "modified".into(),
            },
            PrFileChange {
                filename: "src/main.rs".into(),
                additions: 600,
                deletions: 0,
                status: "modified".into(),
            },
        ];
        let s = format_diff_risk_scan("repo", 1, &files);
        assert!(s.contains("lockfile"));
        assert!(s.contains("large_diff"));
        assert!(s.contains("package-lock.json"));
    }

    #[test]
    fn format_diff_risk_scan_flags_workflow_and_tests_removed() {
        let files = vec![
            PrFileChange {
                filename: ".github/workflows/ci.yml".into(),
                additions: 1,
                deletions: 1,
                status: "modified".into(),
            },
            PrFileChange {
                filename: "tests/foo_test.rs".into(),
                additions: 0,
                deletions: 5,
                status: "removed".into(),
            },
        ];
        let s = format_diff_risk_scan("repo", 1, &files);
        assert!(s.contains("workflow_changed"));
        assert!(s.contains("tests_removed"));
    }

    #[test]
    fn merged_since_date_accepts_iso_date() {
        let d = merged_since_date("2026-01-15").unwrap();
        assert_eq!(d, "2026-01-15");
    }

    #[test]
    fn merged_since_date_rejects_garbage() {
        assert!(merged_since_date("not-a-date").is_err());
    }

    #[test]
    fn merged_since_date_days_integer_falls_back_to_14_on_zero() {
        // 0 or negative -> 14 days. We can't assert exact date due to time,
        // but it should parse and produce a YYYY-MM-DD shape.
        let d = merged_since_date("0").unwrap();
        assert_eq!(d.len(), 10);
        assert_eq!(d.as_bytes().get(4), Some(&b'-'));
    }

    #[test]
    fn base64_decode_roundtrip_ascii() {
        // "Hello, World!" in base64
        let s = base64_decode("SGVsbG8sIFdvcmxkIQ==").unwrap();
        assert_eq!(s, "Hello, World!");
    }
}
