use serde_json::{json, Value};

use crate::error::Result;
use crate::mcp::McpClient;

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
