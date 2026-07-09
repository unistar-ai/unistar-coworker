//! Local Store virtual tools — no GitHub API.
//!
//! Dispatch for harness tools shared by chat. MCP tools stay in `chat_loop` /
//! `github` harness; mutating tools use the approval queue.

use serde_json::Value;

use crate::error::{CoworkerError, Result};
use crate::store::{Approval, Store};

const HARNESS_TOOLS: &[&str] = &["store_list_pending_approvals"];

const DEFAULT_APPROVAL_LIMIT: usize = 20;
const MAX_APPROVAL_LIMIT: usize = 50;

/// Whether `name` is executed locally against [`Store`], not via MCP.
pub fn is_harness_tool(name: &str) -> bool {
    HARNESS_TOOLS.contains(&name)
}

/// Store harness tools (chat catalog + dispatch).
pub fn is_chat_harness_tool(name: &str) -> bool {
    is_harness_tool(name)
}

pub async fn execute_harness_tool(store: &dyn Store, name: &str, args: Value) -> Result<String> {
    match name {
        "store_list_pending_approvals" => format_store_list_pending_approvals(store, &args).await,
        other => Err(CoworkerError::Workflow(format!(
            "unknown harness tool: {other}"
        ))),
    }
}

async fn format_store_list_pending_approvals(store: &dyn Store, args: &Value) -> Result<String> {
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

fn parse_limit_arg(args: &Value, key: &str, default: usize, max: usize) -> usize {
    args.get(key)
        .and_then(|v| v.as_u64())
        .map(|n| n as usize)
        .unwrap_or(default)
        .clamp(1, max)
}

fn format_approval_line(a: &Approval) -> String {
    use crate::store::ApprovalKind;
    let pr = a
        .pr_number
        .map(|n| format!(" #{n}"))
        .unwrap_or_default();
    match a.kind {
        ApprovalKind::McpTool => format!("MCP {:?}{pr}: {}", a.kind, a.description),
        _ => format!("{:?}{pr}: {}", a.kind, a.description),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn harness_tool_registry() {
        assert!(is_harness_tool("store_list_pending_approvals"));
        assert!(!is_harness_tool("pr_get_overview"));
    }
}
