use serde::Deserialize;
use serde_json::Value;

use super::args::{optional_u32, require_str, require_u32};
use super::exec::GhExec;
use crate::error::{CoworkerError, Result};

const DEFAULT_ISSUE_LIMIT: u32 = 20;

#[derive(Debug, Deserialize)]
struct IssueAuthor {
    login: String,
}

#[derive(Debug, Deserialize)]
struct IssueLabel {
    name: String,
}

#[derive(Debug, Deserialize)]
struct IssueItem {
    number: u32,
    title: String,
    author: IssueAuthor,
    state: String,
    labels: Vec<IssueLabel>,
    #[serde(default, rename = "updatedAt")]
    updated_at: String,
}

#[derive(Debug, Deserialize)]
struct IssueDetail {
    number: u32,
    title: String,
    author: IssueAuthor,
    state: String,
    body: String,
    labels: Vec<IssueLabel>,
}

pub async fn issue_list_open(exec: &GhExec, args: &Value) -> Result<String> {
    let repo = require_str(args, "repo")?;
    let limit = optional_u32(args, "limit", DEFAULT_ISSUE_LIMIT);
    let limit_s = limit.to_string();
    let gh_args = [
        "issue",
        "list",
        "-R",
        &repo,
        "--state",
        "open",
        "--limit",
        &limit_s,
        "--json",
        "number,title,author,state,labels,updatedAt",
    ];
    let res = exec.run_retry(&gh_args).await;
    let stdout = GhExec::into_result(res, "failed to list issues")?;
    let issues: Vec<IssueItem> = serde_json::from_str(&stdout).map_err(|e| {
        CoworkerError::Other(anyhow::anyhow!(super::error::format_tool_error(
            super::error::ErrCode::Generic,
            &format!("failed to parse issue list: {e}"),
            "retry",
        )))
    })?;
    if issues.is_empty() {
        return Ok(format!("No open issues in {repo}."));
    }
    let mut lines = vec![format!("{} open issue(s) in {repo}:", issues.len())];
    for i in &issues {
        let labels: Vec<_> = i.labels.iter().map(|l| l.name.as_str()).collect();
        let label_note = if labels.is_empty() {
            String::new()
        } else {
            format!(" [{}]", labels.join(", "))
        };
        lines.push(format!(
            "#{}  {}  @{}  {}  updated:{}{}",
            i.number,
            i.title,
            i.author.login,
            i.state,
            i.updated_at.get(..10).unwrap_or(&i.updated_at),
            label_note
        ));
    }
    Ok(lines.join("\n"))
}

pub async fn issue_get(exec: &GhExec, args: &Value) -> Result<String> {
    let repo = require_str(args, "repo")?;
    let num = require_u32(args, "issue_number")?;
    let num_s = num.to_string();
    let gh_args = [
        "issue",
        "view",
        &num_s,
        "-R",
        &repo,
        "--json",
        "number,title,author,state,body,labels",
    ];
    let res = exec.run_retry(&gh_args).await;
    let stdout = GhExec::into_result(res, "failed to get issue")?;
    let issue: IssueDetail = serde_json::from_str(&stdout).map_err(|e| {
        CoworkerError::Other(anyhow::anyhow!(super::error::format_tool_error(
            super::error::ErrCode::Generic,
            &format!("failed to parse issue: {e}"),
            "retry",
        )))
    })?;
    let labels: Vec<_> = issue.labels.iter().map(|l| l.name.as_str()).collect();
    let body = if issue.body.chars().count() > 4000 {
        format!(
            "{}…\n[truncated]",
            issue.body.chars().take(4000).collect::<String>()
        )
    } else {
        issue.body.clone()
    };
    Ok(format!(
        "Issue #{} {} (@{})\nState: {}\nLabels: {}\n\n{body}",
        issue.number,
        issue.title,
        issue.author.login,
        issue.state,
        if labels.is_empty() {
            "(none)".into()
        } else {
            labels.join(", ")
        }
    ))
}

pub async fn issue_add_label(exec: &GhExec, args: &Value) -> Result<String> {
    let repo = require_str(args, "repo")?;
    let num = require_u32(args, "issue_number")?;
    let label = require_str(args, "label")?;
    let num_s = num.to_string();
    let gh_args = [
        "issue",
        "edit",
        &num_s,
        "-R",
        &repo,
        "--add-label",
        &label,
    ];
    let res = exec.run(&gh_args).await;
    GhExec::into_result(res, "failed to add label")?;
    Ok(super::error::format_tool_ok(&format!(
        "Added label {label:?} to issue #{num} in {repo}."
    )))
}

pub async fn issue_search(exec: &GhExec, args: &Value) -> Result<String> {
    let repo = require_str(args, "repo")?;
    let query = require_str(args, "query")?;
    if query.is_empty() {
        return Err(CoworkerError::Other(anyhow::anyhow!(
            super::error::format_tool_error(
                super::error::ErrCode::Validation,
                "query is empty",
                "pass GitHub issue search terms",
            )
        )));
    }
    let mut limit = optional_u32(args, "limit", DEFAULT_ISSUE_LIMIT);
    if limit == 0 {
        limit = DEFAULT_ISSUE_LIMIT;
    }
    if limit > 50 {
        limit = 50;
    }
    let limit_s = limit.to_string();
    let gh_args = [
        "search",
        "issues",
        &query,
        "--repo",
        &repo,
        "--limit",
        &limit_s,
        "--json",
        "number,title,author,state,labels",
    ];
    let res = exec.run_retry(&gh_args).await;
    let stdout = GhExec::into_result(res, "failed to search issues")?;
    let issues: Vec<IssueItem> = serde_json::from_str(&stdout).map_err(|e| {
        CoworkerError::Other(anyhow::anyhow!(super::error::format_tool_error(
            super::error::ErrCode::Generic,
            &format!("failed to parse issue search: {e}"),
            "retry",
        )))
    })?;
    if issues.is_empty() {
        return Ok(format!("No issues matching {query:?} in {repo}."));
    }
    let mut lines = vec![format!(
        "{} issue(s) matching {query:?} in {repo}:",
        issues.len()
    )];
    for i in &issues {
        let labels: Vec<_> = i.labels.iter().map(|l| l.name.as_str()).collect();
        let label_note = if labels.is_empty() {
            "(none)".into()
        } else {
            labels.join(",")
        };
        lines.push(format!(
            "#{}  {}  @{}  {}  labels:{label_note}",
            i.number,
            i.title,
            i.author.login,
            i.state.to_ascii_lowercase()
        ));
    }
    lines.push("Next: issue_get for full body on a specific number.".into());
    Ok(lines.join("\n"))
}
