use serde_json::{json, Value};

use crate::mcp::registry::McpToolDescriptor;

pub const PROTOCOL_VERSION: &str = "2024-11-05";

pub fn parse_tool_descriptor(tool: Value) -> Option<McpToolDescriptor> {
    let remote_name = tool.get("name")?.as_str()?.to_string();
    let description = tool
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let input_schema = tool
        .get("inputSchema")
        .cloned()
        .unwrap_or_else(|| json!({"type":"object"}));
    let annotations = tool.get("annotations").cloned().unwrap_or(json!({}));
    let read_only_hint = annotations
        .get("readOnlyHint")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let destructive_hint = annotations
        .get("destructiveHint")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    Some(McpToolDescriptor {
        remote_name,
        description,
        input_schema,
        read_only_hint,
        destructive_hint,
    })
}

pub fn format_tool_result(result: &Value) -> String {
    if let Some(content) = result.get("content").and_then(|v| v.as_array()) {
        let mut out = String::new();
        for block in content {
            match block.get("type").and_then(|v| v.as_str()) {
                Some("text") => {
                    if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                        out.push_str(text);
                    }
                }
                Some(other) => {
                    out.push_str(&format!("[{other} block]"));
                }
                None => {}
            }
        }
        if !out.is_empty() {
            return out;
        }
    }
    result.to_string()
}

pub fn format_resource_result(result: &Value) -> String {
    if let Some(contents) = result
        .get("contents")
        .and_then(|v| v.as_array())
        .filter(|a| !a.is_empty())
    {
        let mut out = String::new();
        for item in contents {
            if let Some(text) = item.get("text").and_then(|v| v.as_str()) {
                out.push_str(text);
            } else if let Some(blob) = item.get("blob").and_then(|v| v.as_str()) {
                out.push_str(blob);
            }
        }
        if !out.is_empty() {
            return out;
        }
    }
    format_tool_result(result)
}

pub fn extract_rpc_result(value: &Value, expected_id: u64) -> Option<Result<Value, String>> {
    if value.get("method").is_some() && value.get("id").is_none() {
        return None;
    }
    if value.get("id").and_then(|v| v.as_u64()) != Some(expected_id) {
        return None;
    }
    if let Some(err) = value.get("error") {
        return Some(Err(err.to_string()));
    }
    value
        .get("result")
        .cloned()
        .map(Ok)
        .or(Some(Err("mcp missing result".into())))
}

pub fn parse_sse_messages(body: &str) -> Vec<Value> {
    let mut messages = Vec::new();
    for block in body.split("\n\n") {
        for line in block.lines() {
            let data = line
                .strip_prefix("data:")
                .or_else(|| line.strip_prefix("data: "))
                .map(str::trim)
                .unwrap_or("");
            if data.is_empty() {
                continue;
            }
            if let Ok(value) = serde_json::from_str::<Value>(data) {
                messages.push(value);
            }
        }
    }
    messages
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_sse_messages_reads_data_lines() {
        let body = "event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}\n\n";
        let msgs = parse_sse_messages(body);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].get("id").and_then(|v| v.as_u64()), Some(1));
    }
}
