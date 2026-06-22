use std::collections::HashSet;

use regex::Regex;
use serde::Deserialize;
use serde_json::Value;

use super::args::require_str;
use super::error::{format_tool_error, ErrCode};
use super::exec::GhExec;
use super::pr::{
    format_pr_list_line, format_proverview_batch_line, PullRequest, PullRequestOverviewBatch,
};
use crate::error::{CoworkerError, Result};

const MAX_BATCH_PR_STATUS: usize = 15;
const MAX_BATCH_PR_OVERVIEW: usize = 5;

pub async fn pr_get_status_batch(exec: &GhExec, args: &Value) -> Result<String> {
    let repo = require_str(args, "repo")?;
    let raw_nums = require_str(args, "pr_numbers")?;
    let numbers = parse_pr_number_list(&raw_nums, MAX_BATCH_PR_STATUS)?;

    let (found, missing) = fetch_pr_status_batch(exec, &repo, &numbers).await?;

    let mut lines = vec![format!(
        "Status batch for {repo} ({} requested, {} found):",
        numbers.len(),
        found.len()
    )];
    if numbers.len() == MAX_BATCH_PR_STATUS {
        lines.push(format!("(capped at {MAX_BATCH_PR_STATUS} PRs per call)"));
    }
    for n in &numbers {
        if let Some(pr) = found.get(n) {
            lines.push(format_pr_list_line(pr));
        }
    }
    if !missing.is_empty() {
        let parts: Vec<String> = missing.iter().map(|n| format!("#{n}")).collect();
        lines.push(format!(
            "Missing: {} (not found or not accessible)",
            parts.join(", ")
        ));
    }
    if found.is_empty() {
        lines.push("No PR status returned — verify numbers are open/accessible PRs in this repo.".into());
    }
    Ok(lines.join("\n").trim().to_string())
}

pub async fn pr_get_overview_batch(exec: &GhExec, args: &Value) -> Result<String> {
    let repo = require_str(args, "repo")?;
    let raw_nums = require_str(args, "pr_numbers")?;
    let numbers = parse_pr_number_list(&raw_nums, MAX_BATCH_PR_OVERVIEW)?;

    let (found, missing) = fetch_proverview_batch(exec, &repo, &numbers).await?;

    let mut lines = vec![format!(
        "Overview batch for {repo} ({} requested, {} found):",
        numbers.len(),
        found.len()
    )];
    if numbers.len() == MAX_BATCH_PR_OVERVIEW {
        lines.push(format!(
            "(capped at {MAX_BATCH_PR_OVERVIEW} PRs per call — no failing run IDs in batch)"
        ));
    }
    for n in &numbers {
        if let Some(pr) = found.get(n) {
            lines.push(format_proverview_batch_line(pr));
        }
    }
    if !missing.is_empty() {
        let parts: Vec<String> = missing.iter().map(|n| format!("#{n}")).collect();
        lines.push(format!("Missing: {}", parts.join(", ")));
    }
    if found.is_empty() {
        lines.push("No PR overview returned.".into());
    } else {
        lines.push("Next: pr_get_overview or ci_analyze_pr_failures on PRs with failing CI.".into());
    }
    Ok(lines.join("\n").trim().to_string())
}

fn parse_pr_number_list(raw: &str, max: usize) -> Result<Vec<u32>> {
    static NUM_RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let re = NUM_RE.get_or_init(|| Regex::new(r"^\d+$").unwrap());

    let raw = raw.trim();
    if raw.is_empty() {
        return Err(CoworkerError::Other(anyhow::anyhow!(format_tool_error(
            ErrCode::Validation,
            "pr_numbers is empty",
            "Pass pr_numbers as comma-separated integers, e.g. \"42,43\"",
        ))));
    }

    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for part in raw.split([',', ' ', ';']) {
        let p = part.trim().trim_start_matches('#');
        if p.is_empty() {
            continue;
        }
        if !re.is_match(p) {
            return Err(CoworkerError::Other(anyhow::anyhow!(format_tool_error(
                ErrCode::Validation,
                &format!("invalid PR number {p:?}"),
                "Pass pr_numbers as comma-separated integers",
            ))));
        }
        let n: u32 = p.parse().map_err(|_| {
            CoworkerError::Other(anyhow::anyhow!(format_tool_error(
                ErrCode::Validation,
                &format!("invalid PR number {p:?}"),
                "Pass pr_numbers as comma-separated integers",
            )))
        })?;
        if n == 0 {
            return Err(CoworkerError::Other(anyhow::anyhow!(format_tool_error(
                ErrCode::Validation,
                &format!("invalid PR number {p:?}"),
                "PR numbers must be positive",
            ))));
        }
        if seen.insert(n) {
            out.push(n);
        }
    }
    if out.is_empty() {
        return Err(CoworkerError::Other(anyhow::anyhow!(format_tool_error(
            ErrCode::Validation,
            &format!("no valid PR numbers in {raw:?}"),
            "Pass pr_numbers as comma-separated integers",
        ))));
    }
    out.sort_unstable();
    if max > 0 && out.len() > max {
        out.truncate(max);
    }
    Ok(out)
}

fn split_owner_repo(repo: &str) -> Result<(String, String)> {
    let parts: Vec<&str> = repo.trim().split('/').collect();
    if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
        return Err(CoworkerError::Other(anyhow::anyhow!(format_tool_error(
            ErrCode::Validation,
            &format!("invalid repo {repo:?} (want owner/name)"),
            "Use owner/repo form",
        ))));
    }
    Ok((parts[0].to_string(), parts[1].to_string()))
}

async fn fetch_pr_status_batch(
    exec: &GhExec,
    repo: &str,
    numbers: &[u32],
) -> Result<(std::collections::HashMap<u32, PullRequest>, Vec<u32>)> {
    let (owner, name) = split_owner_repo(repo)?;
    let query = build_pr_status_batch_query(&owner, &name, numbers);
    let query_arg = format!("query={query}");
    let args = ["api", "graphql", "-f", &query_arg];
    let res = exec.run_retry(&args).await;
    let stdout = GhExec::into_result(res, "GraphQL batch PR status failed")?;
    parse_batch_response(&stdout, numbers)
}

async fn fetch_proverview_batch(
    exec: &GhExec,
    repo: &str,
    numbers: &[u32],
) -> Result<(std::collections::HashMap<u32, PullRequestOverviewBatch>, Vec<u32>)> {
    let (owner, name) = split_owner_repo(repo)?;
    let query = build_proverview_batch_query(&owner, &name, numbers);
    let query_arg = format!("query={query}");
    let args = ["api", "graphql", "-f", &query_arg];
    let res = exec.run_retry(&args).await;
    let stdout = GhExec::into_result(res, "GraphQL batch PR overview failed")?;
    parse_overview_batch_response(&stdout, numbers)
}

#[derive(Debug, Deserialize)]
struct GraphqlEnvelope {
    data: GraphqlData,
    #[serde(default)]
    errors: Vec<GraphqlError>,
}

#[derive(Debug, Deserialize)]
struct GraphqlData {
    repository: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct GraphqlError {
    message: String,
}

fn parse_batch_response(
    stdout: &str,
    numbers: &[u32],
) -> Result<(std::collections::HashMap<u32, PullRequest>, Vec<u32>)> {
    let envelope: GraphqlEnvelope = serde_json::from_str(stdout).map_err(|e| {
        CoworkerError::Other(anyhow::anyhow!(format_tool_error(
            ErrCode::Generic,
            &format!("failed to parse GraphQL response: {e}"),
            "retry",
        )))
    })?;
    if let Some(err) = envelope.errors.first() {
        return Err(CoworkerError::Other(anyhow::anyhow!(format_tool_error(
            ErrCode::Generic,
            &format!("GraphQL error: {}", err.message),
            "retry",
        ))));
    }
    let repo_fields: std::collections::HashMap<String, serde_json::Value> =
        serde_json::from_value(envelope.data.repository).map_err(|e| {
            CoworkerError::Other(anyhow::anyhow!(format_tool_error(
                ErrCode::Generic,
                &format!("failed to parse repository batch: {e}"),
                "retry",
            )))
        })?;

    let mut found = std::collections::HashMap::new();
    let mut missing = Vec::new();
    for &n in numbers {
        let key = format!("pr{n}");
        let raw = repo_fields.get(&key);
        let Some(raw) = raw else {
            missing.push(n);
            continue;
        };
        if raw.is_null() {
            missing.push(n);
            continue;
        }
        let mut pr: PullRequest = serde_json::from_value(raw.clone()).unwrap_or_default();
        if pr.number == 0 {
            pr.number = n;
        }
        if pr.number == 0 && pr.title.is_empty() {
            missing.push(n);
        } else {
            found.insert(n, pr);
        }
    }
    Ok((found, missing))
}

fn parse_overview_batch_response(
    stdout: &str,
    numbers: &[u32],
) -> Result<(
    std::collections::HashMap<u32, PullRequestOverviewBatch>,
    Vec<u32>,
)> {
    let envelope: GraphqlEnvelope = serde_json::from_str(stdout).map_err(|e| {
        CoworkerError::Other(anyhow::anyhow!(format_tool_error(
            ErrCode::Generic,
            &format!("failed to parse GraphQL response: {e}"),
            "retry",
        )))
    })?;
    if let Some(err) = envelope.errors.first() {
        return Err(CoworkerError::Other(anyhow::anyhow!(format_tool_error(
            ErrCode::Generic,
            &format!("GraphQL error: {}", err.message),
            "retry",
        ))));
    }
    let repo_fields: std::collections::HashMap<String, serde_json::Value> =
        serde_json::from_value(envelope.data.repository).map_err(|e| {
            CoworkerError::Other(anyhow::anyhow!(format_tool_error(
                ErrCode::Generic,
                &format!("failed to parse repository batch: {e}"),
                "retry",
            )))
        })?;

    let mut found = std::collections::HashMap::new();
    let mut missing = Vec::new();
    for &n in numbers {
        let key = format!("pr{n}");
        let raw = repo_fields.get(&key);
        let Some(raw) = raw else {
            missing.push(n);
            continue;
        };
        if raw.is_null() {
            missing.push(n);
            continue;
        }
        let mut pr: PullRequestOverviewBatch = serde_json::from_value(raw.clone()).unwrap_or_default();
        if pr.number == 0 {
            pr.number = n;
        }
        if pr.number == 0 && pr.title.is_empty() {
            missing.push(n);
        } else {
            found.insert(n, pr);
        }
    }
    Ok((found, missing))
}

fn build_pr_status_batch_query(owner: &str, name: &str, numbers: &[u32]) -> String {
    let mut b = String::from("query { repository(owner: ");
    b.push_str(&serde_json::to_string(owner).unwrap());
    b.push_str(", name: ");
    b.push_str(&serde_json::to_string(name).unwrap());
    b.push_str(") {");
    for &n in numbers {
        b.push_str(&format!(" pr{n}: pullRequest(number: {n}) {{"));
        b.push_str(
            r#"
			number title isDraft reviewDecision mergeable state
			author { login }
			statusCheckRollup {
				__typename
				... on CheckRun { name status conclusion }
				... on StatusContext { context state }
			}
		}"#,
        );
        b.push('}');
    }
    b.push_str(" } }");
    b
}

fn build_proverview_batch_query(owner: &str, name: &str, numbers: &[u32]) -> String {
    let mut b = String::from("query { repository(owner: ");
    b.push_str(&serde_json::to_string(owner).unwrap());
    b.push_str(", name: ");
    b.push_str(&serde_json::to_string(name).unwrap());
    b.push_str(") {");
    for &n in numbers {
        b.push_str(&format!(" pr{n}: pullRequest(number: {n}) {{"));
        b.push_str(
            r#"
			number title isDraft reviewDecision mergeable state
			additions deletions changedFiles
			author { login }
			statusCheckRollup {
				__typename
				... on CheckRun { name status conclusion }
				... on StatusContext { context state }
			}
		}"#,
        );
        b.push('}');
    }
    b.push_str(" } }");
    b
}
