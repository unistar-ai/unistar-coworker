//! Local Store virtual tools — no GitHub API.
//!
//! Dispatch for harness tools shared by chat (and optionally workflows).
//! MCP tools stay in `chat_loop` / `github` harness; mutating tools use the approval queue.

use serde_json::Value;

use crate::agent::context::truncate_chars;
use crate::agent::oncall::build_handoff_markdown;
use crate::error::{CoworkerError, Result};
use crate::store::{Approval, ApprovalKind, Store};

const HARNESS_TOOLS: &[&str] = &[
    "store_get_latest_digest",
    "store_list_pending_approvals",
    "store_get_oncall_handoff",
];

const DEFAULT_APPROVAL_LIMIT: usize = 20;
const MAX_APPROVAL_LIMIT: usize = 50;

/// Whether `name` is executed locally against [`Store`], not via MCP.
pub fn is_harness_tool(name: &str) -> bool {
    HARNESS_TOOLS.contains(&name)
}

/// Store harness or workflow-delegate harness tools (chat catalog + dispatch).
pub fn is_chat_harness_tool(name: &str) -> bool {
    is_harness_tool(name) || crate::agent::workflow_harness::is_workflow_harness_tool(name)
}

const MAX_ONCALL_HANDOFF_CHARS: usize = 6_000;

pub async fn execute_harness_tool(store: &dyn Store, name: &str, args: Value) -> Result<String> {
    match name {
        "store_get_latest_digest" => format_store_latest_digest(store).await,
        "store_list_pending_approvals" => format_store_list_pending_approvals(store, &args).await,
        "store_get_oncall_handoff" => format_store_oncall_handoff(store).await,
        other => Err(CoworkerError::Workflow(format!(
            "unknown harness tool: {other}"
        ))),
    }
}

async fn format_store_latest_digest(store: &dyn Store) -> Result<String> {
    let mut lines = Vec::new();
    if let Some(d) = store.latest_digest().await? {
        lines.push(format!(
            "Latest digest ({}) — needs_attention={} ignorable={} flaky={} policy={} complete={}",
            d.date,
            d.summary.needs_attention,
            d.summary.ignorable,
            d.summary.flaky_candidates,
            d.summary.policy_gates,
            d.summary.complete
        ));
        if !d.body_md.is_empty() {
            let body = if d.body_md.chars().count() > 4000 {
                format!("{}…\n[truncated]", truncate_chars(&d.body_md, 4000))
            } else {
                d.body_md.clone()
            };
            lines.push(String::new());
            lines.push(body);
        }
    } else {
        lines.push("No digest stored yet — run daily-work or another workflow first.".into());
    }

    let pending = store.list_pending_approvals().await?;
    if !pending.is_empty() {
        lines.push(format!("\nPending approvals: {}", pending.len()));
        for a in pending.iter().take(5) {
            lines.push(format!("- {}", format_approval_line(a)));
        }
        if pending.len() > 5 {
            lines.push(format!(
                "… and {} more — use `store_list_pending_approvals` for the full queue",
                pending.len() - 5
            ));
        }
    }

    Ok(lines.join("\n"))
}

async fn format_store_list_pending_approvals(
    store: &dyn Store,
    args: &Value,
) -> Result<String> {
    let limit = parse_limit_arg(args, "limit", DEFAULT_APPROVAL_LIMIT, MAX_APPROVAL_LIMIT);
    let pending = store.list_pending_approvals().await?;
    if pending.is_empty() {
        return Ok("No pending approvals.".into());
    }

    let mut lines = vec![format!("Pending approvals ({})", pending.len())];
    for a in pending.iter().take(limit) {
        lines.push(format!("- {}", format_approval_line(a)));
    }
    if pending.len() > limit {
        lines.push(format!(
            "… {} more not shown (raise `limit` up to {MAX_APPROVAL_LIMIT})",
            pending.len() - limit
        ));
    }
    Ok(lines.join("\n"))
}

async fn format_store_oncall_handoff(store: &dyn Store) -> Result<String> {
    let body = build_handoff_markdown(store).await?;
    if body.chars().count() <= MAX_ONCALL_HANDOFF_CHARS {
        Ok(body)
    } else {
        Ok(format!(
            "{}…\n\n[truncated — run `report oncall` for full export]",
            truncate_chars(&body, MAX_ONCALL_HANDOFF_CHARS)
        ))
    }
}

fn parse_limit_arg(args: &Value, key: &str, default: usize, max: usize) -> usize {
    args.get(key)
        .and_then(json_usize)
        .unwrap_or(default)
        .clamp(1, max)
}

fn json_usize(value: &Value) -> Option<usize> {
    value
        .as_u64()
        .and_then(|n| usize::try_from(n).ok())
        .or_else(|| value.as_str().and_then(|s| s.parse().ok()))
}

fn approval_kind_tool(kind: &ApprovalKind) -> &'static str {
    match kind {
        ApprovalKind::RerunFlaky => "ci_rerun_workflow",
        ApprovalKind::Backport => "pr_create_backport",
        ApprovalKind::PostComment => "pr_post_comment",
        ApprovalKind::IssueAddLabel => "issue_add_label",
        ApprovalKind::WriteFile => "write_file",
        ApprovalKind::EditFile => "edit_file",
        ApprovalKind::BashRun => "bash_run",
        ApprovalKind::PythonRun => "python_run",
        ApprovalKind::McpTool => "mcp_tool",
    }
}

fn format_approval_line(a: &Approval) -> String {
    let tool = approval_kind_tool(&a.kind);
    let mut parts = vec![tool.to_string(), a.repo.clone()];
    if let Some(n) = a.pr_number {
        parts.push(format!("PR #{n}"));
    }
    if let Some(run) = a.run_id {
        parts.push(format!("run {run}"));
    }
    if let Some(ref branch) = a.target_branch {
        parts.push(format!("→ {branch}"));
    }
    if let Some(n) = a.issue_number {
        parts.push(format!("issue #{n}"));
    }
    if let Some(ref label) = a.label {
        parts.push(format!("label `{label}`"));
    }
    let head = parts.join(" | ");
    format!("{head} — {}", truncate_chars(&a.description, 160))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::json::JsonStore;
    use crate::store::{Approval, ApprovalKind, ApprovalStatus, Digest, DigestSummary, Store};
    use chrono::Utc;
    use serde_json::json;
    use uuid::Uuid;

    #[test]
    fn harness_tool_registry() {
        assert!(is_harness_tool("store_get_latest_digest"));
        assert!(is_harness_tool("store_list_pending_approvals"));
        assert!(is_harness_tool("store_get_oncall_handoff"));
        assert!(!is_harness_tool("pr_get_overview"));
        assert_eq!(HARNESS_TOOLS.len(), 3);
    }

    #[tokio::test]
    async fn store_get_latest_digest_empty_store() {
        let dir = tempfile::tempdir().unwrap();
        let store = JsonStore::open(dir.path().to_path_buf()).unwrap();
        let out = execute_harness_tool(&store, "store_get_latest_digest", Value::Null)
            .await
            .unwrap();
        assert!(out.contains("No digest stored yet"));
    }

    #[tokio::test]
    async fn store_get_latest_digest_includes_body_and_pending() {
        let dir = tempfile::tempdir().unwrap();
        let store = JsonStore::open(dir.path().to_path_buf()).unwrap();
        store
            .save_digest(&Digest {
                id: Uuid::new_v4(),
                date: Utc::now().date_naive(),
                summary: DigestSummary {
                    needs_attention: 2,
                    ignorable: 1,
                    flaky_candidates: 0,
                    policy_gates: 0,
                    duration_secs: 3.0,
                    complete: true,
                },
                body_md: "## Needs attention\n- PR #42".into(),
                created_at: Utc::now(),
                skill: None,
            })
            .await
            .unwrap();

        let out = execute_harness_tool(&store, "store_get_latest_digest", Value::Null)
            .await
            .unwrap();
        assert!(out.contains("needs_attention=2"));
        assert!(out.contains("## Needs attention"));
    }

    #[tokio::test]
    async fn store_list_pending_approvals_respects_limit() {
        let dir = tempfile::tempdir().unwrap();
        let store = JsonStore::open(dir.path().to_path_buf()).unwrap();
        for i in 0..3 {
            store
                .push_approval(&Approval {
                    id: Uuid::new_v4(),
                    kind: ApprovalKind::RerunFlaky,
                    repo: "acme/widget".into(),
                    pr_number: Some(i + 1),
                    run_id: Some(100 + i64::from(i)),
                    target_branch: None,
                    incident_id: None,
                    description: format!("rerun PR #{}", i + 1),
                    status: ApprovalStatus::Pending,
                    created_at: Utc::now(),
                    decided_at: None,
                    comment_body: None,
                    issue_number: None,
                    label: None,
                })
                .await
                .unwrap();
        }

        let out = execute_harness_tool(
            &store,
            "store_list_pending_approvals",
            json!({ "limit": 2 }),
        )
        .await
        .unwrap();
        assert!(out.contains("Pending approvals (3)"));
        assert!(out.contains("1 more not shown"));
    }
}
