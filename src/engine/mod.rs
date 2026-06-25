use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::sync::broadcast;

use crate::agent::AgentLoop;
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
pub mod playbook;
pub mod prompt;
pub mod rules;
pub mod scheduler;
pub mod skill;
pub mod skill_routing;
pub mod workflow_registry;

pub use skill_routing::SkillRegistry;
pub mod workflows;

pub use workflow_registry::{require as require_workflow, WORKFLOWS};

pub use prompt::{
    compose_chat_system_prompt, format_session_context_message,
    load_chat_prompt_bundle_for_session, load_classify_skills_for_triage, load_workflow_spec,
    WorkflowSpec, SESSION_CONTEXT_PREFIX,
};
pub use skill::{load_markdown_spec, load_skill_with_base, SkillSpec};

pub struct Engine {
    config: Config,
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
            config,
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

    pub async fn run_workflow(&self, workflow_id: &str) -> Result<String> {
        let agent = AgentLoop::new(
            self.config.clone(),
            Arc::clone(&self.store),
            Arc::clone(&self.github),
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
            Arc::clone(&self.github),
            Arc::clone(&self.mcp),
            id,
            approve,
        )
        .await?;
        self.refresh_store().await?;
        let _ = self.events.send(AppEvent::StatusMessage(msg.clone()));
        Ok(msg)
    }

    /// Lazy-load PR overview for the PRs tab Detail pane (MCP Resource, fallback to tool).
    pub async fn fetch_pr_overview(&self, repo: String, pr_number: u32) {
        use crate::github::helpers::{gh_tool, pr_overview_resource_uri, read_resource};

        use serde_json::json;

        let key = crate::app::AppState::pr_overview_key(&repo, pr_number);
        let uri = pr_overview_resource_uri(&repo, pr_number);
        let body = match read_resource(self.github.as_ref(), &uri).await {
            Ok(text) => text,
            Err(resource_err) => {
                tracing::debug!(
                    "resources/read failed for {uri} ({resource_err}); falling back to pr_get_overview"
                );
                match gh_tool(
                    self.github.as_ref(),
                    "pr_get_overview",
                    json!({ "repo": repo, "pr_number": pr_number }),
                )
                .await
                {
                    Ok(text) => text,
                    Err(e) => format!("## Overview unavailable\n\n{e}"),
                }
            }
        };
        {
            let mut s = self.state.write().await;
            s.pr_overview_fetching = None;
            s.pr_overview_cache.insert(key, body);
        }
        let _ = self
            .events
            .send(AppEvent::PrOverviewReady { repo, pr_number });
    }

    /// Background CI triage for one PR (PRs tab `t`).
    pub fn spawn_triage_pr(self: &Arc<Self>, repo: String, pr_number: u32) {
        let engine = Arc::clone(self);
        tokio::spawn(async move {
            let wf = format!("triage:{repo}#{pr_number}");
            let _ = engine.events.send(AppEvent::WorkflowStarted {
                workflow_id: wf.clone(),
            });
            let result = engine.triage_pr_for_number(&repo, pr_number).await;
            let (ok, message) = match result {
                Ok(outcome) => (true, outcome.full_note()),
                Err(e) => (false, e.to_string()),
            };
            if ok {
                if let Err(e) = engine.refresh_store().await {
                    engine.emit_log("warn", format!("post-triage hydrate: {e}"));
                }
            }
            let _ = engine.events.send(AppEvent::WorkflowFinished {
                workflow_id: wf,
                ok,
                message,
            });
        });
    }

    async fn triage_pr_for_number(
        &self,
        repo: &str,
        pr_number: u32,
    ) -> Result<crate::agent::triage::TriageOutcome> {
        use crate::agent::parse::ParsedPrLine;
        use crate::agent::triage::triage_pr;
        use crate::github::helpers::gh_tool;

        let classify_skills = load_classify_skills_for_triage(&[])?;

        let pr_line = {
            let s = self.state.read().await;
            s.prs
                .iter()
                .find(|p| p.repo == repo && p.number == pr_number)
                .map(|p| ParsedPrLine {
                    number: p.number,
                    title: p.title.clone(),
                    author: p.author.clone(),
                    ci: p.ci_summary.clone(),
                    review: p.review_summary.clone(),
                    is_draft: p.is_draft,
                })
        };

        let pr_line = if let Some(p) = pr_line {
            p
        } else {
            use crate::agent::parse::parse_pr_line;

            let list_text = gh_tool(
                self.github.as_ref(),
                "pr_list_open",
                serde_json::json!({ "repo": repo, "limit": 50 }),
            )
            .await?;
            list_text
                .lines()
                .find_map(|line| {
                    let p = parse_pr_line(line)?;
                    (p.number == pr_number).then_some(p)
                })
                .ok_or_else(|| {
                    crate::error::CoworkerError::Workflow(format!(
                        "PR #{pr_number} not found in {repo}"
                    ))
                })?
        };

        triage_pr(
            &self.config,
            self.github.as_ref(),
            self.llm.as_ref(),
            self.store.as_ref(),
            &classify_skills,
            repo,
            &pr_line,
            Some(&self.events),
        )
        .await
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

    /// Re-measure GitHub harness / LLM latency and reload MCP servers from disk config.
    pub async fn refresh_connectivity_probes(&self) {
        let llm_latency_ms = crate::llm::ollama::probe_latency_ms(&self.config.llm).await;
        let llm_online = llm_latency_ms.is_some();
        let github_latency_ms = if self.github.is_available() {
            crate::github::helpers::probe_github_latency_ms(self.github.as_ref()).await
        } else {
            None
        };
        let github_ok = self.github.is_available();
        let config_path = {
            let s = self.state.read().await;
            s.config_path.clone()
        };
        if let Ok(new_cfg) = Config::load(&config_path) {
            self.mcp.reload_from_config(new_cfg.mcp.clone()).await;
        }
        let mcp_servers = self.mcp.status_snapshot().await;
        let mut s = self.state.write().await;
        s.github_ok = github_ok;
        s.llm_ok = llm_online;
        s.github_latency_ms = github_latency_ms;
        s.llm_latency_ms = llm_latency_ms;
        s.mcp_servers = mcp_servers;
    }
}
