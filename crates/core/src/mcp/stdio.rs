use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::time::timeout;

use crate::config::McpServerConfig;
use crate::error::{CoworkerError, Result};
use crate::mcp::cancel::{cancelled_error, is_cancelled_error, wait_until_cancelled, McpCancel};
use crate::mcp::registry::McpToolDescriptor;
use crate::mcp::rpc::{
    format_resource_result, format_tool_result, parse_tool_descriptor, PROTOCOL_VERSION,
};

pub struct StdioMcpClient {
    child: Child,
    transport: JsonRpcTransport,
}

struct JsonRpcTransport {
    stdin: ChildStdin,
    reader: BufReader<ChildStdout>,
    next_id: u64,
}

impl StdioMcpClient {
    pub async fn connect(server: &McpServerConfig, timeout_secs: u64) -> Result<Self> {
        let command = server.command.as_deref().ok_or_else(|| {
            CoworkerError::Other(anyhow::anyhow!(
                "mcp server {:?} transport stdio requires command",
                server.id
            ))
        })?;
        let mut cmd = Command::new(command);
        cmd.args(&server.args);
        cmd.envs(&server.env);
        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::null());
        cmd.kill_on_drop(true);

        let mut child = cmd.spawn().map_err(|e| {
            CoworkerError::Other(anyhow::anyhow!(
                "mcp server {:?} spawn failed: {e}",
                server.id
            ))
        })?;
        let stdin = child.stdin.take().ok_or_else(|| {
            CoworkerError::Other(anyhow::anyhow!("mcp {:?}: no stdin", server.id))
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            CoworkerError::Other(anyhow::anyhow!("mcp {:?}: no stdout", server.id))
        })?;

        let mut client = Self {
            child,
            transport: JsonRpcTransport {
                stdin,
                reader: BufReader::new(stdout),
                next_id: 0,
            },
        };

        let init_timeout = Duration::from_secs(timeout_secs.max(1));
        timeout(init_timeout, client.initialize(None))
            .await
            .map_err(|_| {
                CoworkerError::Other(anyhow::anyhow!(
                    "mcp server {:?} initialize timed out after {timeout_secs}s",
                    server.id
                ))
            })??;
        Ok(client)
    }

    /// Kill the stdio child and invalidate the transport (chat cancel / drop).
    pub fn abort_in_flight(&mut self) {
        let _ = self.child.start_kill();
    }

    async fn request_cancellable(
        &mut self,
        method: &str,
        params: Value,
        cancel: McpCancel,
    ) -> Result<Value> {
        match self.transport.request(method, params, &cancel).await {
            Err(e) if is_cancelled_error(&e) => {
                self.abort_in_flight();
                Err(e)
            }
            other => other,
        }
    }

    async fn initialize(&mut self, cancel: McpCancel) -> Result<()> {
        let params = json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": {
                "name": "unistar-coworker",
                "version": env!("CARGO_PKG_VERSION"),
            }
        });
        let _ = self
            .request_cancellable("initialize", params, cancel)
            .await?;
        self.transport
            .notify("notifications/initialized", json!({}))
            .await?;
        Ok(())
    }

    pub async fn list_tools(&mut self, cancel: McpCancel) -> Result<Vec<McpToolDescriptor>> {
        let result = self
            .request_cancellable("tools/list", json!({}), cancel)
            .await?;
        let tools = result
            .get("tools")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        Ok(tools
            .into_iter()
            .filter_map(parse_tool_descriptor)
            .collect())
    }

    pub async fn call_tool(
        &mut self,
        name: &str,
        arguments: Value,
        timeout_secs: u64,
        cancel: McpCancel,
    ) -> Result<String> {
        let params = json!({
            "name": name,
            "arguments": arguments,
        });
        let call_timeout = Duration::from_secs(timeout_secs.max(1));
        let result = timeout(
            call_timeout,
            self.request_cancellable("tools/call", params, cancel),
        )
        .await
        .map_err(|_| {
            CoworkerError::Other(anyhow::anyhow!(
                "mcp tools/call {name} timed out after {timeout_secs}s"
            ))
        })??;
        if result.get("isError").and_then(|v| v.as_bool()) == Some(true) {
            let msg = format_tool_result(&result);
            return Err(CoworkerError::Other(anyhow::anyhow!(msg)));
        }
        Ok(format_tool_result(&result))
    }

    pub async fn read_resource(
        &mut self,
        uri: &str,
        timeout_secs: u64,
        cancel: McpCancel,
    ) -> Result<String> {
        let params = json!({ "uri": uri });
        let call_timeout = Duration::from_secs(timeout_secs.max(1));
        let result = timeout(
            call_timeout,
            self.request_cancellable("resources/read", params, cancel),
        )
        .await
        .map_err(|_| {
            CoworkerError::Other(anyhow::anyhow!(
                "mcp resources/read {uri} timed out after {timeout_secs}s"
            ))
        })??;
        Ok(format_resource_result(&result))
    }
}

impl JsonRpcTransport {
    async fn request(&mut self, method: &str, params: Value, cancel: &McpCancel) -> Result<Value> {
        self.next_id += 1;
        let id = self.next_id;
        let msg = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        self.write_message(&msg).await?;
        loop {
            tokio::select! {
                biased;
                _ = wait_until_cancelled(cancel) => {
                    return Err(cancelled_error());
                }
                line = self.read_line() => {
                    let line = line?;
                    let value: Value = serde_json::from_str(&line).map_err(|e| {
                        CoworkerError::Other(anyhow::anyhow!("mcp invalid json: {e}; line={line:?}"))
                    })?;
                    if value.get("method").is_some() && value.get("id").is_none() {
                        tracing::debug!("mcp notification: {}", value);
                        continue;
                    }
                    if value.get("id").and_then(|v| v.as_u64()) == Some(id) {
                        if let Some(err) = value.get("error") {
                            return Err(CoworkerError::Other(anyhow::anyhow!(
                                "mcp rpc error: {err}"
                            )));
                        }
                        return value
                            .get("result")
                            .cloned()
                            .ok_or_else(|| CoworkerError::Other(anyhow::anyhow!("mcp missing result")));
                    }
                }
            }
        }
    }

    async fn notify(&mut self, method: &str, params: Value) -> Result<()> {
        let msg = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        self.write_message(&msg).await
    }

    async fn write_message(&mut self, msg: &Value) -> Result<()> {
        let line = serde_json::to_string(msg)?;
        self.stdin.write_all(line.as_bytes()).await?;
        self.stdin.write_all(b"\n").await?;
        self.stdin.flush().await?;
        Ok(())
    }

    async fn read_line(&mut self) -> Result<String> {
        let mut line = String::new();
        self.reader
            .read_line(&mut line)
            .await
            .map_err(|e| CoworkerError::Other(anyhow::anyhow!("mcp read failed: {e}")))?;
        if line.trim().is_empty() {
            return Err(CoworkerError::Other(anyhow::anyhow!(
                "mcp server closed stdout"
            )));
        }
        Ok(line)
    }
}

#[cfg(test)]
mod tests {
    use std::process::Stdio;

    use super::*;

    #[tokio::test]
    async fn abort_in_flight_kills_sleep_child() {
        let mut cmd = Command::new("sleep");
        cmd.arg("120");
        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::null());
        let mut child = cmd.spawn().expect("spawn sleep");
        let stdin = child.stdin.take().expect("stdin");
        let stdout = child.stdout.take().expect("stdout");
        let mut client = StdioMcpClient {
            child,
            transport: JsonRpcTransport {
                stdin,
                reader: BufReader::new(stdout),
                next_id: 0,
            },
        };
        client.abort_in_flight();
        tokio::time::sleep(Duration::from_millis(50)).await;
        let status = client.child.wait().await.expect("wait");
        assert!(!status.success());
    }
}
