use std::sync::Arc;

use tokio::sync::broadcast;

use crate::agent::AgentLoop;
use crate::app::{hydrate_from_store, AppEvent, SharedState};
use crate::config::Config;
use crate::error::Result;
use crate::llm::LlmClient;
use crate::mcp::{spawn_mcp, McpClient};
use crate::store::Store;

pub mod scheduler;
pub mod skill;
pub mod workflows;
pub mod approvals;

pub use skill::{load_skill, Skill};

pub struct Engine {
    config: Config,
    store: Arc<dyn Store>,
    mcp: Arc<dyn McpClient>,
    llm: Arc<LlmClient>,
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
        let llm_online = crate::llm::ollama::probe(&config.llm).await;
        let llm = Arc::new(LlmClient::new(config.llm.clone(), llm_online));
        {
            let mut s = state.write().await;
            s.mcp_ok = mcp.is_available();
            s.llm_ok = llm_online;
        }
        Self {
            config,
            store,
            mcp,
            llm,
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
            Arc::clone(&self.llm),
            self.events.clone(),
            Arc::clone(&self.state),
        );
        let result = agent.run_workflow(workflow_id).await;
        self.refresh_store().await?;
        result
    }

    pub async fn is_busy(&self) -> bool {
        self.state.read().await.engine_busy
    }

    pub async fn decide_approval(&self, id: &uuid::Uuid, approve: bool) -> Result<String> {
        let msg = approvals::process_decision(
            Arc::clone(&self.store),
            Arc::clone(&self.mcp),
            id,
            approve,
        )
        .await?;
        self.refresh_store().await?;
        let _ = self.events.send(AppEvent::StatusMessage(msg.clone()));
        Ok(msg)
    }

    pub fn spawn_background(self: Arc<Self>) {
        tokio::spawn(async move {
            if let Err(e) = self.refresh_store().await {
                tracing::warn!("initial hydrate: {e}");
            }
        });
    }

    pub fn spawn_scheduler(self: Arc<Self>) {
        let scheduler = scheduler::Scheduler::from_config(&self.config);
        scheduler.spawn(self);
    }
}
