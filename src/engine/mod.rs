use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::sync::broadcast;

use crate::agent::AgentLoop;
use crate::app::{hydrate_from_store, AppEvent, SharedState};
use crate::config::Config;
use crate::error::Result;
use crate::llm::LlmClient;
use crate::mcp::{spawn_mcp, McpClient};
use crate::store::{LogLine, Store};

pub mod scheduler;
pub mod skill;
pub mod prompt;
pub mod workflows;
pub mod approvals;
pub mod chat;
pub mod rules;
pub mod playbook;

pub use skill::{load_markdown_spec, load_skill_with_base, AgentSpec, SkillSpec};
pub use prompt::{
    compose_system_prompt, load_chat_prompt_bundle, load_classify_skills_for_triage,
    load_tools_doc_with_preferred, load_workflow_spec, WorkflowSpec,
};

pub struct Engine {
    config: Config,
    store: Arc<dyn Store>,
    mcp: Arc<dyn McpClient>,
    llm: Arc<LlmClient>,
    events: broadcast::Sender<AppEvent>,
    state: SharedState,
    chat_cancel: Arc<AtomicBool>,
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
        let engine = Self {
            config,
            store,
            mcp,
            llm,
            events,
            state,
            chat_cancel: Arc::new(AtomicBool::new(false)),
        };
        if !engine.mcp.is_available() {
            engine.emit_log(
                "warn",
                "unistar-mcp unavailable — set mcp.command and GH_TOKEN",
            );
        }
        engine
    }

    /// Internal log line for TUI Logs tab (+ stderr when not in TUI mode).
    pub fn emit_log(&self, level: &str, message: impl Into<String>) {
        let message = message.into();
        let _ = self.events.send(AppEvent::LogLine(LogLine {
            ts: chrono::Utc::now(),
            level: level.to_string(),
            message: message.clone(),
        }));
        match level {
            "warn" => tracing::warn!("{message}"),
            "error" => tracing::error!("{message}"),
            _ => tracing::info!("{message}"),
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

    /// Request cancellation of the in-flight chat turn (TUI Esc).
    pub fn request_chat_cancel(&self) {
        self.chat_cancel.store(true, Ordering::Relaxed);
    }

    fn reset_chat_cancel(&self) {
        self.chat_cancel.store(false, Ordering::Relaxed);
    }

    pub fn chat_cancel_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.chat_cancel)
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

    pub async fn acknowledge_main_alert(&self, id: &uuid::Uuid) -> Result<()> {
        self.store.acknowledge_main_alert(id).await?;
        self.refresh_store().await?;
        let _ = self.events.send(AppEvent::StatusMessage(format!(
            "main alert {id} acknowledged"
        )));
        Ok(())
    }

    pub async fn reclassify_flaky(&self, fingerprint: &str, as_flaky: bool) -> Result<u32> {
        use crate::store::Classification;
        let classification = if as_flaky {
            Classification::UserFlaky
        } else {
            Classification::UserReal
        };
        let updated = self.store.reclassify_flaky(fingerprint, classification).await?;
        self.refresh_store().await?;
        Ok(updated)
    }

    pub fn spawn_background(self: Arc<Self>) {
        tokio::spawn(async move {
            if let Err(e) = self.refresh_store().await {
                self.emit_log("warn", format!("initial hydrate: {e}"));
            }
        });
    }

    pub fn spawn_scheduler(self: Arc<Self>) {
        let scheduler = scheduler::Scheduler::from_config(&self.config);
        scheduler.spawn(self);
    }
}
