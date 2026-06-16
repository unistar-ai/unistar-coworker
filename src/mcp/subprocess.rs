use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::Mutex;
use tokio::time::timeout;

use crate::config::McpConfig;
use crate::error::{CoworkerError, Result};
use crate::mcp::McpClient;

const MCP_REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

/// Minimal MCP stdio client for unistar-mcp lazy meta tools (v0.1).
pub struct SubprocessMcp {
    stdin: Mutex<ChildStdin>,
    reader: Mutex<BufReader<tokio::process::ChildStdout>>,
    _child: Mutex<Child>,
    next_id: Mutex<u64>,
}

impl SubprocessMcp {
    pub async fn try_spawn(cfg: &McpConfig) -> Result<Self> {
        let mut cmd = Command::new(&cfg.command);
        cmd.args(&cfg.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        for (k, v) in &cfg.env {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn().map_err(|e| {
            CoworkerError::Other(anyhow::anyhow!("spawn {}: {e}", cfg.command))
        })?;
        let stdin = child.stdin.take().ok_or_else(|| {
            CoworkerError::Other(anyhow::anyhow!("mcp stdin missing"))
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            CoworkerError::Other(anyhow::anyhow!("mcp stdout missing"))
        })?;
        let client = Self {
            stdin: Mutex::new(stdin),
            reader: Mutex::new(BufReader::new(stdout)),
            _child: Mutex::new(child),
            next_id: Mutex::new(1),
        };
        client.initialize().await?;
        Ok(client)
    }

    async fn initialize(&self) -> Result<()> {
        let _ = self
            .request(
                "initialize",
                serde_json::json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {},
                    "clientInfo": { "name": "unistar-coworker", "version": "0.1.0" }
                }),
            )
            .await?;
        self.notify("notifications/initialized", serde_json::json!({}))
            .await
    }

    async fn next_id(&self) -> u64 {
        let mut id = self.next_id.lock().await;
        let v = *id;
        *id += 1;
        v
    }

    async fn notify(&self, method: &str, params: serde_json::Value) -> Result<()> {
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        let mut line = serde_json::to_string(&msg)?;
        line.push('\n');
        let mut stdin = self.stdin.lock().await;
        stdin.write_all(line.as_bytes()).await?;
        stdin.flush().await?;
        Ok(())
    }

    async fn request(&self, method: &str, params: serde_json::Value) -> Result<serde_json::Value> {
        timeout(MCP_REQUEST_TIMEOUT, self.request_inner(method, params))
            .await
            .map_err(|_| {
                CoworkerError::Other(anyhow::anyhow!(
                    "mcp request timed out after {}s ({method})",
                    MCP_REQUEST_TIMEOUT.as_secs()
                ))
            })?
    }

    async fn request_inner(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let id = self.next_id().await;
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        let mut line = serde_json::to_string(&msg)?;
        line.push('\n');
        {
            let mut stdin = self.stdin.lock().await;
            stdin.write_all(line.as_bytes()).await?;
            stdin.flush().await?;
        }

        loop {
            let mut reader = self.reader.lock().await;
            let mut buf = String::new();
            let n = reader.read_line(&mut buf).await?;
            if n == 0 {
                return Err(CoworkerError::Other(anyhow::anyhow!(
                    "mcp closed stdout while waiting for {method} response"
                )));
            }
            if buf.trim().is_empty() {
                continue;
            }
            let v: serde_json::Value = serde_json::from_str(buf.trim())?;
            if v.get("id").and_then(|x| x.as_u64()) == Some(id) {
                if let Some(err) = v.get("error") {
                    return Err(CoworkerError::Other(anyhow::anyhow!("mcp error: {err}")));
                }
                return Ok(v
                    .get("result")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null));
            }
        }
    }
}

#[async_trait]
impl McpClient for SubprocessMcp {
    fn is_available(&self) -> bool {
        true
    }

    async fn tool_call(&self, tool: &str, args: serde_json::Value) -> Result<String> {
        let result = self
            .request(
                "tools/call",
                serde_json::json!({
                    "name": tool,
                    "arguments": args,
                }),
            )
            .await?;

        extract_tool_text(&result)
    }
}

fn extract_tool_text(result: &serde_json::Value) -> Result<String> {
    let text = collect_tool_text(result)?;
    if result.get("isError").and_then(|v| v.as_bool()) == Some(true) {
        return Err(CoworkerError::Other(anyhow::anyhow!(text.trim().to_string())));
    }
    Ok(text)
}

fn collect_tool_text(result: &serde_json::Value) -> Result<String> {
    if let Some(content) = result.get("content").and_then(|c| c.as_array()) {
        let mut out = String::new();
        for item in content {
            if item.get("type").and_then(|t| t.as_str()) == Some("text") {
                if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                    out.push_str(text);
                    out.push('\n');
                }
            }
        }
        if !out.is_empty() {
            return Ok(out);
        }
    }
    Ok(result.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_tool_text_honors_is_error() {
        let result = serde_json::json!({
            "content": [{
                "type": "text",
                "text": "failed to list pull requests: HTTP 504: 504 Gateway Timeout"
            }],
            "isError": true
        });
        let err = extract_tool_text(&result).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("504"));
        assert!(msg.contains("failed to list"));
    }

    #[test]
    fn extract_tool_text_ok_when_not_error() {
        let result = serde_json::json!({
            "content": [{
                "type": "text",
                "text": "open PR(s) in acme/widget (2):\n#1 a @x CI:passing review:none"
            }],
            "isError": false
        });
        let text = extract_tool_text(&result).unwrap();
        assert!(text.contains("open PR(s)"));
    }
}
