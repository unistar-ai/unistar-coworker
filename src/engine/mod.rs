use std::sync::Arc;

use tokio::sync::broadcast;

use crate::agent::AgentLoop;
use crate::app::{hydrate_from_store, AppEvent, SharedState};
use crate::config::Config;
use crate::error::Result;
use crate::mcp::{spawn_mcp, McpClient};
use crate::store::Store;

pub mod scheduler;
pub mod workflows;

pub struct Engine {
    config: Config,
    store: Arc<dyn Store>,
    mcp: Arc<dyn McpClient>,
    events: broadcast::Sender<AppEvent>,
    state: SharedState,
}

impl Engine {
    pub async fn new(
        config: Config,
        store: Arc<dyn Store>,
        events: broadcast::Sender<AppEvent>,
        state: SharedState,
    ) -> Self {
        let mcp = spawn_mcp(&config).await;
        {
            let mut s = state.write().await;
            s.mcp_ok = mcp.is_available();
            s.llm_ok = crate::llm::ollama::probe(&config.llm).await;
        }
        Self {
            config,
            store,
            mcp,
            events,
            state,
        }
    }

    pub async fn refresh_store(&self) -> Result<()> {
        hydrate_from_store(&self.state, self.store.as_ref()).await?;
        let _ = self.events.send(AppEvent::StoreUpdated);
        Ok(())
    }

    pub async fn run_workflow(&self, workflow_id: &str) -> Result<String> {
        let agent = AgentLoop::new(
            self.config.clone(),
            Arc::clone(&self.store),
            Arc::clone(&self.mcp),
            self.events.clone(),
            Arc::clone(&self.state),
        );
        let result = agent.run_workflow(workflow_id).await;
        self.refresh_store().await?;
        result
    }

    pub fn spawn_background(self: Arc<Self>) {
        tokio::spawn(async move {
            if let Err(e) = self.refresh_store().await {
                tracing::warn!("initial hydrate: {e}");
            }
        });
    }
}
