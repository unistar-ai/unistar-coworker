use serde_json::{json, Value};

use super::discovery;
use super::harness::GithubHarness;
use crate::error::{CoworkerError, Result};

pub const CHAT_GITHUB_TOOLS: &[&str] = &[
    "pr_get_overview",
    "pr_list_changed_files",
    "pr_list_open",
    "pr_get_status",
    "pr_get_merge_blockers",
    "pr_get_diff",
    "ci_analyze_pr_failures",
    "ci_get_run_summary",
    "ci_list_runs",
    "ci_branch_health",
    "ci_get_failed_logs",
    "ci_failure_fingerprint",
    "policy_classify_failure",
    "ci_compare_runs",
    "ci_list_external_checks",
    "repo_get_info",
    "pr_list_merged",
    "pr_list_waiting_review",
    "issue_get",
    "alert_list_open",
];

/// Call a harness GitHub tool (same entry point as former `gh_tool`).
pub async fn gh_tool(harness: &GithubHarness, name: &str, args: Value) -> Result<String> {
    if discovery::is_meta_tool(name) {
        harness.call_tool(name, args).await
    } else {
        harness
            .call_tool(
                "tool_call",
                json!({ "name": name, "args": args }),
            )
            .await
    }
}

pub async fn gh_tool_with_retry(harness: &GithubHarness, name: &str, args: Value) -> Result<String> {
    match gh_tool(harness, name, args.clone()).await {
        Ok(text) => Ok(text),
        Err(e) if is_transient_error(&e) => gh_tool(harness, name, args).await,
        Err(e) => Err(e),
    }
}

pub async fn read_resource(harness: &GithubHarness, uri: &str) -> Result<String> {
    harness.read_resource(uri).await
}

pub fn pr_overview_resource_uri(repo: &str, pr_number: u32) -> String {
    let (owner, name) = repo.split_once('/').unwrap_or((repo, "_"));
    format!("github://pull/{owner}/{name}/{pr_number}/overview")
}

pub fn is_transient_error(err: &CoworkerError) -> bool {
    let msg = err.to_string().to_ascii_lowercase();
    msg.contains("error: transient")
        || msg.contains("http 504")
        || msg.contains("http 503")
        || msg.contains("http 502")
        || msg.contains("gateway timeout")
        || msg.contains("timed out")
}

pub fn mcp_text_indicates_failure(output: &str) -> bool {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return true;
    }
    if trimmed.starts_with("OK:") {
        return false;
    }
    if trimmed.starts_with("ERROR:") {
        return true;
    }
    crate::agent::context::tool_body_header_indicates_failure(trimmed)
}

pub async fn probe_github_latency_ms(harness: &GithubHarness) -> Option<u128> {
    if !harness.is_available() {
        return None;
    }
    let start = std::time::Instant::now();
    match harness.call_tool("tool_list", json!({})).await {
        Ok(_) => Some(start.elapsed().as_millis()),
        Err(_) => None,
    }
}

pub fn chat_github_probe_tools() -> &'static [&'static str] {
    CHAT_GITHUB_TOOLS
}

pub async fn warn_if_github_tools_missing(harness: &GithubHarness) {
    if harness.is_available() {
        return;
    }
    tracing::debug!(
        "GitHub probe tools ({}) unavailable until `gh` is ready",
        chat_github_probe_tools().len()
    );
}
