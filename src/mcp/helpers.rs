use serde_json::{json, Value};

use crate::error::Result;
use crate::mcp::McpClient;

/// Tools chat relies on; missing entries mean an outdated unistar-mcp binary.
pub const CHAT_MCP_TOOLS: &[&str] = &[
    "pr_get_overview",
    "pr_list_changed_files",
    "pr_list_open",
    "pr_get_status",
    "ci_analyze_pr_failures",
];

/// Call a real unistar-mcp tool through lazy-mode `tool_call`.
pub async fn lazy_tool(client: &dyn McpClient, name: &str, args: Value) -> Result<String> {
    client
        .tool_call(
            "tool_call",
            json!({
                "name": name,
                "args": args,
            }),
        )
        .await
}

pub fn parse_tool_names_from_list(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.contains("available:") {
                return None;
            }
            let name = line.split('—').next()?.trim();
            if name.is_empty() {
                return None;
            }
            if name.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                return None;
            }
            Some(name.to_string())
        })
        .collect()
}

pub fn missing_mcp_tools(available: &[String]) -> Vec<&'static str> {
    CHAT_MCP_TOOLS
        .iter()
        .copied()
        .filter(|required| !available.iter().any(|a| a == required))
        .collect()
}

pub async fn probe_mcp_tool_names(client: &dyn McpClient) -> Result<Vec<String>> {
    let list = lazy_tool(client, "tool_list", json!({})).await?;
    Ok(parse_tool_names_from_list(&list))
}

pub async fn warn_if_mcp_tools_missing(client: &dyn McpClient) {
    match probe_mcp_tool_names(client).await {
        Ok(names) => {
            let missing = missing_mcp_tools(&names);
            if !missing.is_empty() {
                tracing::warn!(
                    missing = ?missing,
                    "unistar-mcp binary is outdated — rebuild: cd unistar-mcp && go build -o unistar-mcp ./cmd, then restart coworker"
                );
            }
        }
        Err(e) => tracing::debug!("mcp tool_list probe skipped: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_tool_list_lines() {
        let text = "20 tool(s) available:\npr_list_open — List open\npr_get_overview — Snapshot";
        let names = parse_tool_names_from_list(text);
        assert!(names.contains(&"pr_list_open".to_string()));
        assert!(names.contains(&"pr_get_overview".to_string()));
    }

    #[test]
    fn missing_tools_reported() {
        let avail = vec!["pr_list_open".into(), "pr_get_status".into()];
        let missing = missing_mcp_tools(&avail);
        assert!(missing.contains(&"pr_get_overview"));
        assert!(missing.contains(&"pr_list_changed_files"));
    }
}
