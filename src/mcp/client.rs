use serde_json::Value;

use crate::config::{McpServerConfig, McpTransport};
use crate::error::Result;
use crate::mcp::cancel::McpCancel;
use crate::mcp::http::HttpMcpClient;
use crate::mcp::registry::McpToolDescriptor;
use crate::mcp::stdio::StdioMcpClient;

pub enum McpClient {
    Stdio(StdioMcpClient),
    Http(HttpMcpClient),
}

impl McpClient {
    pub async fn connect(server: &McpServerConfig, timeout_secs: u64) -> Result<Self> {
        match server.transport {
            McpTransport::Stdio => {
                StdioMcpClient::connect(server, timeout_secs)
                    .await
                    .map(McpClient::Stdio)
            }
            McpTransport::Http => {
                HttpMcpClient::connect(server, timeout_secs)
                    .await
                    .map(McpClient::Http)
            }
        }
    }

    pub fn abort_in_flight(&mut self) {
        match self {
            Self::Stdio(client) => client.abort_in_flight(),
            Self::Http(_) => {}
        }
    }

    pub fn is_stdio(&self) -> bool {
        matches!(self, Self::Stdio(_))
    }

    pub async fn list_tools(&mut self, cancel: McpCancel) -> Result<Vec<McpToolDescriptor>> {
        match self {
            Self::Stdio(client) => client.list_tools(cancel).await,
            Self::Http(client) => client.list_tools(cancel).await,
        }
    }

    pub async fn call_tool(
        &mut self,
        name: &str,
        arguments: Value,
        timeout_secs: u64,
        cancel: McpCancel,
    ) -> Result<String> {
        match self {
            Self::Stdio(client) => {
                client
                    .call_tool(name, arguments, timeout_secs, cancel)
                    .await
            }
            Self::Http(client) => {
                client
                    .call_tool(name, arguments, timeout_secs, cancel)
                    .await
            }
        }
    }

    pub async fn read_resource(
        &mut self,
        uri: &str,
        timeout_secs: u64,
        cancel: McpCancel,
    ) -> Result<String> {
        match self {
            Self::Stdio(client) => client.read_resource(uri, timeout_secs, cancel).await,
            Self::Http(client) => client.read_resource(uri, timeout_secs, cancel).await,
        }
    }
}
