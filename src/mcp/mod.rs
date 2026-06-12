use async_trait::async_trait;

use crate::error::{CoworkerError, Result};

pub mod helpers;
mod subprocess;

pub use subprocess::SubprocessMcp;

#[async_trait]
pub trait McpClient: Send + Sync {
    fn is_available(&self) -> bool;
    async fn tool_call(&self, tool: &str, args: serde_json::Value) -> Result<String>;
}

pub async fn spawn_mcp(config: &crate::config::Config) -> std::sync::Arc<dyn McpClient> {
    match SubprocessMcp::try_spawn(&config.mcp).await {
        Ok(client) => std::sync::Arc::new(client),
        Err(e) => {
            tracing::warn!("MCP spawn failed ({e}); using offline stub");
            std::sync::Arc::new(OfflineMcp)
        }
    }
}

struct OfflineMcp;

#[async_trait]
impl McpClient for OfflineMcp {
    fn is_available(&self) -> bool {
        false
    }

    async fn tool_call(&self, _tool: &str, _args: serde_json::Value) -> Result<String> {
        Err(CoworkerError::Other(anyhow::anyhow!(
            "unistar-mcp not running (install/build unistar-mcp and ensure GH_TOKEN)"
        )))
    }
}
