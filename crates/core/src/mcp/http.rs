use std::collections::HashMap;
use std::time::Duration;

use reqwest::header::{ACCEPT, CONTENT_TYPE};
use reqwest::{Client, StatusCode};
use serde_json::{json, Value};
use tokio::time::timeout;

use crate::config::McpServerConfig;
use crate::error::{CoworkerError, Result};
use crate::mcp::cancel::{cancelled_error, wait_until_cancelled, McpCancel};
use crate::mcp::registry::McpToolDescriptor;
use crate::mcp::rpc::{
    extract_rpc_result, format_resource_result, format_tool_result, parse_sse_messages,
    parse_tool_descriptor, PROTOCOL_VERSION,
};

const MCP_SESSION_ID: &str = "mcp-session-id";
const MCP_PROTOCOL_VERSION: &str = "mcp-protocol-version";

pub struct HttpMcpClient {
    http: Client,
    url: String,
    extra_headers: HashMap<String, String>,
    session_id: Option<String>,
    protocol_version: String,
    next_id: u64,
}

impl HttpMcpClient {
    pub async fn connect(server: &McpServerConfig, timeout_secs: u64) -> Result<Self> {
        let url = server.url.as_deref().ok_or_else(|| {
            CoworkerError::Other(anyhow::anyhow!(
                "mcp server {:?} transport http requires url",
                server.id
            ))
        })?;
        let http = Client::builder()
            .timeout(Duration::from_secs(timeout_secs.max(1)))
            .build()
            .map_err(|e| CoworkerError::Other(anyhow::anyhow!("mcp http client: {e}")))?;

        let mut client = Self {
            http,
            url: url.to_string(),
            extra_headers: server.headers.clone(),
            session_id: None,
            protocol_version: PROTOCOL_VERSION.to_string(),
            next_id: 0,
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

    async fn initialize(&mut self, cancel: McpCancel) -> Result<()> {
        let params = json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": {
                "name": "unistar-coworker",
                "version": env!("CARGO_PKG_VERSION"),
            }
        });
        let result = self.request("initialize", params, &cancel).await?;
        if let Some(pv) = result.get("protocolVersion").and_then(|v| v.as_str()) {
            self.protocol_version = pv.to_string();
        }
        self.notify("notifications/initialized", json!({})).await?;
        Ok(())
    }

    pub async fn list_tools(&mut self, cancel: McpCancel) -> Result<Vec<McpToolDescriptor>> {
        let result = self.request("tools/list", json!({}), &cancel).await?;
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
        let result = timeout(call_timeout, self.request("tools/call", params, &cancel))
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
            self.request("resources/read", params, &cancel),
        )
        .await
        .map_err(|_| {
            CoworkerError::Other(anyhow::anyhow!(
                "mcp resources/read {uri} timed out after {timeout_secs}s"
            ))
        })??;
        Ok(format_resource_result(&result))
    }

    async fn request(&mut self, method: &str, params: Value, cancel: &McpCancel) -> Result<Value> {
        self.next_id += 1;
        let id = self.next_id;
        let msg = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        let response = self.post(&msg, cancel).await?;
        self.capture_session_id(&response);
        let status = response.status();
        if status == StatusCode::ACCEPTED {
            return Err(CoworkerError::Other(anyhow::anyhow!(
                "mcp http unexpected 202 for request {method}"
            )));
        }
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(CoworkerError::Other(anyhow::anyhow!(
                "mcp http {method} failed: HTTP {status} {body}"
            )));
        }
        self.parse_response(response, id, cancel).await
    }

    async fn notify(&mut self, method: &str, params: Value) -> Result<()> {
        let msg = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        let response = self.post(&msg, &None).await?;
        self.capture_session_id(&response);
        let status = response.status();
        if status.is_success() || status == StatusCode::ACCEPTED {
            return Ok(());
        }
        let body = response.text().await.unwrap_or_default();
        Err(CoworkerError::Other(anyhow::anyhow!(
            "mcp http notify {method} failed: HTTP {status} {body}"
        )))
    }

    async fn post(&self, msg: &Value, cancel: &McpCancel) -> Result<reqwest::Response> {
        let url = self.url.clone();
        let http = self.http.clone();
        let extra_headers = self.extra_headers.clone();
        let session_id = self.session_id.clone();
        let protocol_version = self.protocol_version.clone();
        let body = msg.clone();

        let mut handle = tokio::spawn(async move {
            let mut req = http
                .post(&url)
                .header(ACCEPT, "application/json, text/event-stream")
                .header(CONTENT_TYPE, "application/json")
                .header(MCP_PROTOCOL_VERSION, protocol_version)
                .json(&body);
            if let Some(session_id) = session_id {
                req = req.header(MCP_SESSION_ID, session_id);
            }
            for (key, value) in &extra_headers {
                req = req.header(key, value);
            }
            req.send().await
        });

        tokio::select! {
            biased;
            _ = wait_until_cancelled(cancel) => {
                handle.abort();
                Err(cancelled_error())
            }
            joined = &mut handle => joined
                .map_err(|e| CoworkerError::Other(anyhow::anyhow!("mcp http task join: {e}")))?
                .map_err(|e| CoworkerError::Other(anyhow::anyhow!("mcp http post failed: {e}"))),
        }
    }

    fn capture_session_id(&mut self, response: &reqwest::Response) {
        if let Some(session_id) = response
            .headers()
            .get(MCP_SESSION_ID)
            .and_then(|v| v.to_str().ok())
        {
            self.session_id = Some(session_id.to_string());
        }
    }

    async fn parse_response(
        &self,
        response: reqwest::Response,
        expected_id: u64,
        cancel: &McpCancel,
    ) -> Result<Value> {
        let mut handle = tokio::spawn(async move {
            let content_type = response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_ascii_lowercase();

            if content_type.starts_with("application/json") {
                let value: Value = response.json().await.map_err(|e| {
                    CoworkerError::Other(anyhow::anyhow!("mcp http invalid json: {e}"))
                })?;
                return HttpMcpClient::finish_rpc(value, expected_id);
            }

            if content_type.starts_with("text/event-stream") {
                let body = response.text().await.map_err(|e| {
                    CoworkerError::Other(anyhow::anyhow!("mcp http sse read failed: {e}"))
                })?;
                for value in parse_sse_messages(&body) {
                    if let Some(outcome) = extract_rpc_result(&value, expected_id) {
                        return outcome.map_err(|e| CoworkerError::Other(anyhow::anyhow!(e)));
                    }
                }
                return Err(CoworkerError::Other(anyhow::anyhow!(
                    "mcp http sse response missing id {expected_id}"
                )));
            }

            Err(CoworkerError::Other(anyhow::anyhow!(
                "mcp http unexpected content-type: {content_type}"
            )))
        });

        tokio::select! {
            biased;
            _ = wait_until_cancelled(cancel) => {
                handle.abort();
                Err(cancelled_error())
            }
            joined = &mut handle => joined
                .map_err(|e| CoworkerError::Other(anyhow::anyhow!("mcp http parse task: {e}")))?,
        }
    }

    fn finish_rpc(value: Value, expected_id: u64) -> Result<Value> {
        match extract_rpc_result(&value, expected_id) {
            Some(Ok(result)) => Ok(result),
            Some(Err(err)) => Err(CoworkerError::Other(anyhow::anyhow!(
                "mcp rpc error: {err}"
            ))),
            None => Err(CoworkerError::Other(anyhow::anyhow!(
                "mcp http json response missing id {expected_id}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use axum::http::{header, StatusCode};
    use axum::response::IntoResponse;
    use axum::routing::post;
    use axum::{Json, Router};
    use serde_json::{json, Value};

    use super::*;
    use crate::config::{McpApprovalConfig, McpExposeConfig, McpServerConfig, McpTransport};

    fn test_http_server(url: &str) -> McpServerConfig {
        McpServerConfig {
            id: "mock".into(),
            enabled: true,
            transport: McpTransport::Http,
            command: None,
            args: vec![],
            env: HashMap::new(),
            url: Some(url.into()),
            headers: HashMap::new(),
            expose: McpExposeConfig::default(),
            approval: McpApprovalConfig::default(),
            startup: None,
            timeout_secs: None,
            skills: vec![],
        }
    }

    #[tokio::test]
    async fn http_client_handles_json_responses() {
        let app = Router::new().route(
            "/mcp",
            post(|Json(body): Json<Value>| async move {
                let method = body.get("method").and_then(|v| v.as_str()).unwrap_or("");
                let id = body.get("id").cloned();
                match method {
                    "initialize" => (
                        StatusCode::OK,
                        [(header::CONTENT_TYPE, "application/json")],
                        Json(json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": {
                                "protocolVersion": "2024-11-05",
                                "capabilities": {},
                                "serverInfo": { "name": "mock", "version": "0" }
                            }
                        })),
                    )
                        .into_response(),
                    "notifications/initialized" => StatusCode::ACCEPTED.into_response(),
                    "tools/list" => (
                        StatusCode::OK,
                        [(header::CONTENT_TYPE, "application/json")],
                        Json(json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": {
                                "tools": [{
                                    "name": "ping",
                                    "description": "pong",
                                    "inputSchema": { "type": "object" }
                                }]
                            }
                        })),
                    )
                        .into_response(),
                    _ => StatusCode::NOT_FOUND.into_response(),
                }
            }),
        );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let url = format!("http://{addr}/mcp");
        let server = test_http_server(&url);
        let mut client = HttpMcpClient::connect(&server, 5).await.expect("connect");
        let tools = client.list_tools(None).await.expect("list");
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].remote_name, "ping");
    }
}
