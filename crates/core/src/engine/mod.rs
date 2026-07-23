use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};

use tokio::sync::broadcast;

use crate::app::{hydrate_from_store, AppEvent, SharedState};
use crate::config::Config;
use crate::error::Result;
use crate::github::{spawn_github, GithubHarness};
use crate::llm::LlmClient;
use crate::mcp::{spawn_mcp_pool, McpPool};
use crate::store::{LogLine, Store};

pub mod approvals;
pub mod chat;
pub mod embedded_prompts;
pub mod prompt;
pub mod rules;
pub mod skill;
pub mod skill_routing;

pub use skill_routing::SkillRegistry;

pub use prompt::{
    compose_chat_system_prompt, format_session_context_message,
    load_chat_prompt_bundle_for_session, SESSION_CONTEXT_PREFIX,
};
pub use skill::{load_markdown_spec, load_skill_with_base, SkillSpec};

pub struct Engine {
    config: RwLock<Config>,
    store: Arc<dyn Store>,
    github: Arc<GithubHarness>,
    mcp: Arc<McpPool>,
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
        let github = spawn_github(&config).await;
        let mcp = spawn_mcp_pool(&config).await;
        let llm_latency_ms = crate::llm::ollama::probe_latency_ms(&config.llm).await;
        let llm_online = llm_latency_ms.is_some();
        let github_latency_ms = if github.is_available() {
            crate::github::helpers::probe_github_latency_ms(github.as_ref()).await
        } else {
            None
        };
        let llm = Arc::new(LlmClient::new(config.llm.clone(), llm_online));
        {
            let mut s = state.write().await;
            s.github_ok = github.is_available();
            s.llm_ok = llm_online;
            s.github_latency_ms = github_latency_ms;
            s.llm_latency_ms = llm_latency_ms;
            s.mcp_servers = mcp.status_snapshot().await;
        }
        let engine = Self {
            config: RwLock::new(config),
            store,
            github,
            mcp,
            llm,
            events,
            state,
            chat_cancel: Arc::new(AtomicBool::new(false)),
        };
        if !engine.github.is_available() {
            engine.emit_log(
                "warn",
                "GitHub harness unavailable — set github.gh_command and GH_TOKEN",
            );
        }
        for server in engine.mcp.status_snapshot().await {
            if server.connected {
                engine.emit_log(
                    "info",
                    format!(
                        "mcp[{}]: connected ({} tools)",
                        server.id, server.tool_count
                    ),
                );
            } else if server.last_error.is_some() {
                engine.emit_log(
                    "warn",
                    format!(
                        "mcp[{}]: {}",
                        server.id,
                        server.last_error.as_deref().unwrap_or("offline")
                    ),
                );
            }
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

    pub async fn decide_approval(
        &self,
        id: &uuid::Uuid,
        approve: bool,
        decision_reason: Option<&str>,
    ) -> Result<String> {
        let config = self.config.read().expect("config lock").clone();
        let msg = approvals::process_decision(
            Arc::clone(&self.store),
            Arc::clone(&self.github),
            Arc::clone(&self.mcp),
            &config,
            id,
            approve,
            decision_reason,
        )
        .await?;
        self.refresh_store().await?;
        let _ = self.events.send(AppEvent::StatusMessage(msg.clone()));
        Ok(msg)
    }

    pub fn spawn_background(self: Arc<Self>) {
        tokio::spawn(async move {
            if let Err(e) = self.refresh_store().await {
                self.emit_log("warn", format!("initial hydrate: {e}"));
            }
        });
    }

    /// Re-measure GitHub harness / LLM latency and reload MCP servers from disk config.
    pub async fn refresh_connectivity_probes(&self) {
        let config_path = {
            let s = self.state.read().await;
            s.config_path.clone()
        };
        if let Ok(new_cfg) = Config::load(&config_path) {
            self.apply_config_reload(new_cfg).await;
            return;
        }
        let llm = self.config.read().expect("config lock").llm.clone();
        let llm_latency_ms = crate::llm::ollama::probe_latency_ms(&llm).await;
        let llm_online = llm_latency_ms.is_some();
        let github_latency_ms = if self.github.is_available() {
            crate::github::helpers::probe_github_latency_ms(self.github.as_ref()).await
        } else {
            None
        };
        let github_ok = self.github.is_available();
        let mcp_servers = self.mcp.status_snapshot().await;
        let mut s = self.state.write().await;
        s.github_ok = github_ok;
        s.llm_ok = llm_online;
        s.github_latency_ms = github_latency_ms;
        s.llm_latency_ms = llm_latency_ms;
        s.mcp_servers = mcp_servers;
    }

    /// Switch the active LLM preset at runtime (persists to `{config}.llm-profile` sidecar).
    pub async fn switch_llm_profile(&self, name: &str) -> Result<()> {
        {
            let s = self.state.read().await;
            if s.chat_busy {
                return Err(crate::error::CoworkerError::Config(
                    "cannot switch LLM profile while chat is busy".into(),
                ));
            }
            if s.engine_busy {
                return Err(crate::error::CoworkerError::Config(
                    "cannot switch LLM profile while a background task is running".into(),
                ));
            }
        }
        let config_path = self.state.read().await.config_path.clone();
        let mut new_cfg = self.config.read().expect("config lock").clone();
        new_cfg.switch_llm_profile(name)?;
        Config::write_llm_profile_sidecar(std::path::Path::new(&config_path), name)?;
        self.apply_config_reload(new_cfg).await;
        let model = self.config.read().expect("config lock").llm.model.clone();
        self.emit_log("info", format!("LLM profile → {name} ({model})"));
        Ok(())
    }

    /// Hot-reload config, LLM, and MCP servers from disk (triggered by SIGHUP
    /// or `POST /api/reload`). Skills and the chat prompt are re-read from disk
    /// on every chat turn, so they pick up changes automatically after this.
    pub async fn reload_all(&self) {
        let config_path = { self.state.read().await.config_path.clone() };
        match Config::load(&config_path) {
            Ok(new_cfg) => {
                self.apply_config_reload(new_cfg).await;
                self.emit_log("info", "hot reload: config reloaded");
            }
            Err(e) => {
                self.emit_log("warn", format!("hot reload: config load failed: {e}"));
            }
        }
    }

    async fn apply_config_reload(&self, new_cfg: Config) {
        let llm_latency_ms = crate::llm::ollama::probe_latency_ms(&new_cfg.llm).await;
        let llm_online = llm_latency_ms.is_some();
        self.llm.reconfigure(new_cfg.llm.clone(), llm_online);
        self.mcp.reload_from_config(new_cfg.mcp.clone()).await;
        let github_latency_ms = if self.github.is_available() {
            crate::github::helpers::probe_github_latency_ms(self.github.as_ref()).await
        } else {
            None
        };
        let github_ok = self.github.is_available();
        let mcp_servers = self.mcp.status_snapshot().await;
        self.config
            .write()
            .expect("config lock")
            .clone_from(&new_cfg);
        let mut s = self.state.write().await;
        s.config = new_cfg;
        s.github_ok = github_ok;
        s.llm_ok = llm_online;
        s.github_latency_ms = github_latency_ms;
        s.llm_latency_ms = llm_latency_ms;
        s.mcp_servers = mcp_servers;
    }
}
