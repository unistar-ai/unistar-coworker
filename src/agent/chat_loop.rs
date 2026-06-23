use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Utc;
use futures_util::future::join_all;
use serde_json::{json, Value};
use tokio::sync::broadcast;
use tokio::time::{self, MissedTickBehavior};
use uuid::Uuid;

use crate::agent::bash_tool;
use crate::agent::file_edit_tool;
use crate::agent::python_tool;
use crate::agent::review_gate::{format_review_rejection_description, ReviewGateOutcome};
use crate::agent::harness_errors::agent_validation_error;
use crate::agent::web_browser_tool;
use crate::agent::budget::TokenBudget;
use crate::agent::chat_discovery::ChatDiscoveryState;
use crate::agent::context::{
    estimate_message_tokens, estimate_tools_tokens, format_system_for_context_panel,
    format_tool_approval_pending_message, format_tool_context_message,
    format_tools_for_context_panel, harness_nudge_base, history_token_budget,
    message_budget_for_tools, pack_session_history_with_llm, skill_body_for_context_panel,
    tool_names_from_definitions, trim_llm_messages_with_llm, trim_system_content, truncate_chars,
};
use crate::engine::SkillSpec;
use crate::agent::file_tools;
use crate::agent::harness_tools;
use crate::agent::hooks::{HookRunner, TurnContext};
use crate::agent::workflow_harness::{self, WorkflowHarnessCtx};
use crate::agent::parse::parse_failing_runs;
use crate::agent::runtime_context::{
    build_message_focus_lines, build_workspace_git_summary, load_workspace_agents_md,
    plan_runtime_context,
    RuntimeContextInput,
};
use crate::agent::tool_catalog;
use crate::app::{append_audit, AppEvent};
use crate::config::{BashToolConfig, ChatToolMode, Config, PythonToolConfig};
use crate::engine::{
    approvals, compose_chat_system_prompt, format_session_context_message,
    load_chat_prompt_bundle_for_session,
    SkillRegistry,
};
use crate::error::{CoworkerError, Result};
use crate::llm::chat::{
    reply_premature_for_task, reply_premature_nudge, ChatAgentStep, LlmToolCall,
    ResolvedToolCall,
};
use crate::llm::{ChatAgentAction, ChatStepOptions, LlmClient, LlmTurnMessage};
use crate::github::helpers::{gh_tool, gh_tool_with_retry, read_resource};
use crate::github::{effective_chat_tool_mode, GithubHarness};
use tokio::sync::Mutex;
use crate::store::{Approval, ApprovalKind, ApprovalStatus, ChatMessage, ChatRole, Store};

const MUTATING_TOOLS: &[&str] = &[
    "ci_rerun_workflow",
    "pr_create_backport",
    "pr_post_comment",
    "issue_add_label",
];

#[derive(Debug, Clone)]
pub struct ContextLine {
    /// TUI label — may differ from the LLM API role (e.g. tool results are API `user`).
    pub display_role: String,
    pub content: String,
    pub tokens: u32,
}

#[derive(Debug, Clone)]
pub struct ContextSkillBlock {
    pub name: String,
    pub body: String,
    pub tokens: u32,
}

#[derive(Debug, Clone, Default)]
pub struct ContextSnapshot {
    pub turn: u32,
    /// Estimated tokens in the current LLM message list.
    pub message_tokens: u32,
    /// Estimated tokens in native `tools[]` for the current/next LLM step.
    pub tools_tokens: u32,
    /// Readable tool schema text (matches API `tools[]` payload).
    pub tools_body: String,
    /// Tool function names exposed on this step.
    pub tool_names: Vec<String>,
    /// Technique skills injected via the system prompt (`## Techniques`).
    pub skill_blocks: Vec<ContextSkillBlock>,
    /// Sum of [`ContextSkillBlock::tokens`] (included in `message_tokens`).
    pub skills_tokens: u32,
    /// Input budget for messages + tools (context_limit minus fixed reserves).
    pub input_budget: u32,
    /// Model context window from config (llm.context_limit).
    pub context_limit: u32,
    pub message_count: usize,
    pub messages: Vec<ContextLine>,
    pub runtime_context_revision: Option<u64>,
}

impl ContextSnapshot {
    pub fn total_tokens(&self) -> u32 {
        self.message_tokens.saturating_add(self.tools_tokens)
    }
}

#[derive(Debug, Clone)]
pub enum ChatProgress {
    TurnThinking {
        turn: u32,
        elapsed_secs: u64,
    },
    ToolStart {
        name: String,
        args_short: String,
    },
    /// Heartbeat while a readonly MCP tool is in flight (elapsed / paging hint).
    ToolProgress {
        name: String,
        detail: String,
    },
    ToolDone {
        name: String,
        args_short: String,
        ok: bool,
        elapsed_ms: u128,
        /// Capped body for TUI expand (see `chat_tool_outputs`).
        output_preview: String,
    },
    /// In-progress native tool call label while streaming.
    ToolPending {
        label: String,
    },
    ApprovalQueued {
        approval_id: Uuid,
        session_id: Uuid,
        tool_name: String,
        tool_args_json: String,
        description: String,
    },
    /// Mutating tool auto-approved or resolved without the TUI popup.
    ApprovalResolved {
        approval_id: Uuid,
        tool_name: String,
        approved: bool,
        detail: String,
    },
    /// Summarizing streamed thinking via think=false LLM before the next step.
    ReasoningCompressing,
    /// Incremental assistant reply text while streaming.
    AssistantPartial {
        text: String,
    },
    /// Ollama thinking tokens (internal reasoning — not the final answer).
    ReasoningPartial {
        text: String,
    },
    /// Trimmed LLM messages sent on the next step (for TUI context panel).
    ContextSnapshot(ContextSnapshot),
    /// Harness blocked a repeated tool call before execution.
    DuplicateToolBlocked {
        tool_name: String,
        args_short: String,
        attempt: u32,
    },
    /// Harness rejected a tool call before execution (missing args, bad name, etc.).
    HarnessNudge {
        retry: u32,
        preview: String,
    },
    /// Reasoning summary materialized and persisted to the session store.
    ReasoningSummary {
        preview: String,
        /// Full reasoning body for expandable transcript rows.
        body: String,
    },
    /// Transient skill / MCP flow in Messages (live only — like reasoning).
    ActivityFlow {
        kind: ActivityFlowKind,
        text: String,
    },
    /// Clear transient activity flow (tool/skill step finished).
    ActivityFlowClear,
}

/// Skill load vs MCP meta-tool trace shown transiently in the Messages pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityFlowKind {
    Skill,
    Github,
}

/// Live-only skill/MCP flow body for the Messages tail.
#[derive(Debug, Clone)]
pub struct ChatActivityFlow {
    pub kind: ActivityFlowKind,
    pub text: String,
}

/// Tools whose start/done rows are shown as transient activity, not persistent ✓/→ lines.
pub fn is_flow_activity_tool(name: &str) -> bool {
    matches!(
        name,
        "skill_load"
            | "tool_list"
            | "tool_list_category"
            | "tool_search"
            | "tool_describe"
            | "tool_call"
            | "resource_read"
    )
}

fn activity_flow_kind_for_tool(name: &str) -> ActivityFlowKind {
    if matches!(name, "skill_load") {
        ActivityFlowKind::Skill
    } else {
        ActivityFlowKind::Github
    }
}

fn emit_activity_flow(
    progress: &Option<broadcast::Sender<AppEvent>>,
    kind: ActivityFlowKind,
    text: impl Into<String>,
) {
    let body = text.into();
    if body.trim().is_empty() {
        return;
    }
    emit_progress(
        progress,
        ChatProgress::ActivityFlow {
            kind,
            text: body,
        },
    );
}

fn emit_activity_flow_clear(progress: &Option<broadcast::Sender<AppEvent>>) {
    emit_progress(progress, ChatProgress::ActivityFlowClear);
}

pub fn format_skill_bootstrap_flow(skills: &[crate::engine::SkillSpec]) -> String {
    if skills.is_empty() {
        return String::new();
    }
    let mut lines = vec!["Preparing skills for this question:".to_string()];
    for skill in skills {
        if skill.tool_refs.is_empty() {
            lines.push(format!("  • {}", skill.name));
        } else {
            lines.push(format!(
                "  • {} — warm {}",
                skill.name,
                skill.tool_refs.join(", ")
            ));
        }
    }
    lines.join("\n")
}

fn format_flow_tool_start(name: &str, args: &Value) -> String {
    let args_short = format_tool_args_short(args);
    let header = match name {
        "tool_call" => {
            let inner = args
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            format!("tool_call → {inner}")
        }
        "skill_load" => {
            let skill = args.get("name").and_then(|v| v.as_str()).unwrap_or("?");
            format!("skill_load → {skill}")
        }
        "resource_read" => "resource_read".to_string(),
        _ => name.to_string(),
    };
    if args_short.is_empty() {
        header
    } else {
        format!("{header}\n  args: {args_short}")
    }
}

fn format_flow_tool_done(name: &str, args: &Value, ok: bool, preview: &str) -> String {
    let mut text = format_flow_tool_start(name, args);
    let mark = if ok { "ok" } else { "failed" };
    text.push_str(&format!("\n  → {mark}"));
    let snippet = preview
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or(preview);
    let snippet = truncate_chars(snippet, 240);
    if !snippet.trim().is_empty() {
        text.push_str(&format!("\n  {snippet}"));
    }
    text
}

/// Max harness-only LLM retries per user turn (missing args, malformed JSON, etc.).
const MAX_HARNESS_CORRECTIONS: u32 = 10;

impl ChatProgress {
    pub fn show_in_log(&self) -> bool {
        !matches!(
            self,
            Self::TurnThinking { .. }
                | Self::AssistantPartial { .. }
                | Self::ReasoningPartial { .. }
                | Self::ReasoningCompressing
                | Self::ContextSnapshot(_)
                | Self::ToolPending { .. }
                | Self::ToolProgress { .. }
                | Self::ActivityFlow { .. }
                | Self::ActivityFlowClear
        )
    }

    pub fn display_line(&self) -> String {
        match self {
            Self::ContextSnapshot(_) => String::new(),
            Self::TurnThinking { .. } => "  … thinking".into(),
            Self::AssistantPartial { .. } | Self::ReasoningPartial { .. } => String::new(),
            Self::ToolPending { .. } => String::new(),
            Self::ToolProgress { .. } => String::new(),
            Self::ToolStart { name, args_short } => {
                if args_short.is_empty() {
                    format!("  → {name}")
                } else {
                    format!("  → {name}({args_short})")
                }
            }
            Self::ToolDone {
                name,
                args_short,
                ok,
                elapsed_ms,
                output_preview: _,
            } => {
                let mark = if *ok { "✓" } else { "✗" };
                if args_short.is_empty() {
                    format!("  {mark} {name} ({elapsed_ms}ms)")
                } else {
                    format!("  {mark} {name}({args_short}) ({elapsed_ms}ms)")
                }
            }
            Self::ApprovalQueued {
                approval_id,
                tool_name,
                ..
            } => format!("  ⏳ approval pending: {tool_name} ({approval_id})"),
            Self::ApprovalResolved {
                tool_name,
                approved,
                ..
            } => {
                let mark = if *approved { "✓" } else { "✗" };
                format!("  {mark} approval resolved: {tool_name}")
            }
            Self::ReasoningCompressing => "  … summarizing reasoning".into(),
            Self::DuplicateToolBlocked {
                tool_name,
                args_short,
                attempt,
            } => {
                if args_short.is_empty() {
                    format!("  ⚠ duplicate {tool_name} (attempt {attempt})")
                } else {
                    format!("  ⚠ duplicate {tool_name}({args_short}) (attempt {attempt})")
                }
            }
            Self::HarnessNudge { retry, preview } => {
                format!("  ⚠ harness retry {retry}: {preview}")
            }
            Self::ReasoningSummary { preview, .. } => format!("  … reasoning: {preview}"),
            Self::ActivityFlow { .. } | Self::ActivityFlowClear => String::new(),
        }
    }

    pub fn status_text(&self) -> String {
        match self {
            Self::ContextSnapshot(_) => String::new(),
            Self::ActivityFlow { kind, .. } => match kind {
                ActivityFlowKind::Skill => "chat: loading skill…".into(),
                ActivityFlowKind::Github => "chat: GitHub…".into(),
            },
            Self::ActivityFlowClear => String::new(),
            Self::ReasoningCompressing => "chat: summarizing reasoning…".into(),
            Self::TurnThinking { turn, elapsed_secs } => {
                format!("chat thinking (step {turn}, {elapsed_secs}s)…")
            }
            Self::AssistantPartial { .. } => "chat: streaming reply…".into(),
            Self::ReasoningPartial { .. } => "chat: reasoning…".into(),
            Self::ToolPending { label } => format!("chat: {label}…"),
            Self::ToolProgress { name, detail } => format!("chat: {name} ({detail})…"),
            Self::ToolStart { name, .. } => format!("chat: {name}…"),
            Self::ToolDone { name, .. } => format!("chat: {name} done"),
            Self::ApprovalQueued { tool_name, .. } => {
                format!("chat: approval pending — confirm in popup ({tool_name})")
            }
            Self::ApprovalResolved {
                tool_name,
                approved,
                ..
            } => {
                if *approved {
                    format!("chat: approval auto-approved ({tool_name})")
                } else {
                    format!("chat: approval failed ({tool_name})")
                }
            }
            Self::DuplicateToolBlocked {
                tool_name, attempt, ..
            } => {
                format!("chat: duplicate {tool_name} (attempt {attempt})")
            }
            Self::HarnessNudge { retry, .. } => {
                format!("chat: harness correction {retry}")
            }
            Self::ReasoningSummary { .. } => String::new(),
        }
    }
}

pub fn format_tool_args_short(args: &Value) -> String {
    let Some(map) = args.as_object() else {
        return String::new();
    };
    if map.is_empty() {
        return String::new();
    }
    let mut parts: Vec<String> = map
        .iter()
        .take(3)
        .map(|(key, value)| format!("{key}={}", format_arg_value(value)))
        .collect();
    if map.len() > 3 {
        parts.push("…".into());
    }
    parts.join(", ")
}

fn format_arg_value(value: &Value) -> String {
    match value {
        Value::String(s) => truncate_chars(s, 28),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => "null".into(),
        other => truncate_chars(&other.to_string(), 28),
    }
}

fn emit_progress(progress: &Option<broadcast::Sender<AppEvent>>, event: ChatProgress) {
    if let Some(tx) = progress {
        let _ = tx.send(AppEvent::ChatProgress(event));
    }
}

pub fn build_context_snapshot(
    messages: &[LlmTurnMessage],
    turn: u32,
    token_budget: &TokenBudget,
    native_tools: &[Value],
    loaded_skills: &[SkillSpec],
    runtime_panel: Option<(&str, u64)>,
) -> ContextSnapshot {
    let message_tokens = crate::agent::context::estimate_messages_tokens(messages);
    let tools_tokens = estimate_tools_tokens(native_tools);
    let tools_body = format_tools_for_context_panel(native_tools);
    let tool_names = tool_names_from_definitions(native_tools);
    let skill_blocks: Vec<ContextSkillBlock> = loaded_skills
        .iter()
        .map(|s| {
            let body = skill_body_for_context_panel(&s.body);
            let tokens = crate::agent::context::estimate_tokens(&body);
            ContextSkillBlock {
                name: s.name.clone(),
                body,
                tokens,
            }
        })
        .collect();
    let skills_tokens = skill_blocks.iter().map(|s| s.tokens).sum();
    let lines: Vec<ContextLine> = messages
        .iter()
        .map(|m| {
            let raw = crate::agent::context::format_llm_message_for_context_panel(m);
            let content = if m.role == "system" {
                format_system_for_context_panel(&raw)
            } else if m.role == "user"
                && raw.trim_start().starts_with(crate::engine::SESSION_CONTEXT_PREFIX)
            {
                if let Some((full, _)) = runtime_panel {
                    crate::engine::format_session_context_message(full)
                } else {
                    raw
                }
            } else {
                raw
            };
            ContextLine {
                display_role: context_display_role(m.role, &m.content),
                content,
                tokens: estimate_message_tokens(m),
            }
        })
        .collect();
    ContextSnapshot {
        turn,
        message_tokens,
        tools_tokens,
        tools_body,
        tool_names,
        skill_blocks,
        skills_tokens,
        input_budget: token_budget.input_budget(),
        context_limit: token_budget.context_limit,
        message_count: messages.len(),
        messages: lines,
        runtime_context_revision: runtime_panel.map(|(_, rev)| rev),
    }
}

/// Human-readable role for the context panel (API role alone is misleading).
pub fn context_display_role(api_role: &str, content: &str) -> String {
    if api_role == "system" {
        return "system".into();
    }
    let trimmed = content.trim_start();
    if trimmed.starts_with(crate::engine::SESSION_CONTEXT_PREFIX) {
        return "context".into();
    }
    if api_role == "assistant" {
        return "assistant".into();
    }
    if api_role == "tool" {
        return "tool".into();
    }
    if trimmed.starts_with("tool_result(")
        || trimmed.starts_with("tool_error(")
        || trimmed.starts_with("tool_approval_pending(")
        || trimmed.starts_with("[tool_result ")
        || trimmed.starts_with("[summarized tool_result ")
    {
        return "tool".into();
    }
    if trimmed.starts_with("[agent reasoning summary]") {
        return "reasoning".into();
    }
    if trimmed.starts_with("[earlier ")
        || trimmed.contains("omitted from context")
        || trimmed.starts_with("Identical `")
        || trimmed.starts_with("Same tool call repeated")
        || trimmed.starts_with("Tool `")
        || trimmed.starts_with("You pasted multiple tool")
        || trimmed.starts_with("Malformed tool call:")
        || trimmed.starts_with("action:reply looked")
        || trimmed.starts_with("Your reply looked")
        || trimmed.starts_with("Your reply must")
        || trimmed.starts_with("action:reply must be")
        || trimmed.starts_with("You replied without")
        || trimmed.starts_with("Tool budget exhausted")
        || trimmed.starts_with("Invalid tool_name")
        || trimmed.starts_with("Unknown tool_name")
        || trimmed.contains("Did you mean `")
        || trimmed.starts_with("Mutating tool `")
        || trimmed.starts_with("Reached the ")
    {
        return "harness".into();
    }
    "user".into()
}

async fn native_tools_for_session(
    discovery: &Arc<Mutex<ChatDiscoveryState>>,
    tool_mode: ChatToolMode,
) -> Vec<Value> {
    let state = discovery.lock().await;
    tool_catalog::ToolCatalog::new()
        .native_tool_definitions_for_session(tool_mode, &state.warmed_tools)
}

async fn emit_context_snapshot(
    progress: &Option<broadcast::Sender<AppEvent>>,
    messages: &[LlmTurnMessage],
    turn: u32,
    token_budget: &TokenBudget,
    discovery: &Arc<Mutex<ChatDiscoveryState>>,
    tool_mode: ChatToolMode,
    runtime_panel: Option<(&str, u64)>,
) {
    let native_tools = native_tools_for_session(discovery, tool_mode).await;
    let loaded_skills = {
        let state = discovery.lock().await;
        state.loaded_skill_specs()
    };
    emit_progress(
        progress,
        ChatProgress::ContextSnapshot(build_context_snapshot(
            messages,
            turn,
            token_budget,
            &native_tools,
            &loaded_skills,
            runtime_panel,
        )),
    );
}

fn append_assistant_to_llm_context(llm_messages: &mut Vec<LlmTurnMessage>, message: &str) {
    let trimmed = message.trim();
    if trimmed.is_empty() {
        return;
    }
    llm_messages.push(LlmTurnMessage::new("assistant", trimmed.to_string()));
}

#[derive(Debug, Clone)]
pub struct ToolCallSummary {
    pub tool_name: String,
    pub output: String,
}

impl ToolCallSummary {
    pub fn preview(&self, max: usize) -> String {
        if self.output.chars().count() <= max {
            return self.output.clone();
        }
        format!("{}…", self.output.chars().take(max).collect::<String>())
    }
}

#[derive(Debug, Clone)]
pub struct ChatTurnInput {
    pub session_id: Option<Uuid>,
    pub user_message: String,
    pub progress: Option<broadcast::Sender<AppEvent>>,
    pub cancel: Option<Arc<AtomicBool>>,
    /// Continue a paused turn after the user approves or denies a mutating tool.
    pub resume: Option<ResumeChatAfterApproval>,
}

#[derive(Debug, Clone)]
pub struct ResumeChatAfterApproval {
    pub approval_id: Uuid,
    pub approved: bool,
    pub detail: String,
    pub tool_name: String,
    pub tool_args: Value,
}

pub fn is_chat_cancelled(err: &CoworkerError) -> bool {
    matches!(err, CoworkerError::Workflow(msg) if msg == "chat cancelled")
}

fn chat_cancelled_error() -> CoworkerError {
    CoworkerError::Workflow("chat cancelled".into())
}

fn chat_cancel_requested(cancel: &Option<Arc<AtomicBool>>) -> bool {
    cancel
        .as_ref()
        .is_some_and(|flag| flag.load(Ordering::Relaxed))
}

fn ensure_chat_not_cancelled(cancel: &Option<Arc<AtomicBool>>) -> Result<()> {
    if chat_cancel_requested(cancel) {
        Err(chat_cancelled_error())
    } else {
        Ok(())
    }
}

async fn wait_chat_cancel(cancel: Arc<AtomicBool>) {
    while !cancel.load(Ordering::Relaxed) {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn race_chat_cancel<T, F>(cancel: Option<Arc<AtomicBool>>, fut: F) -> Result<T>
where
    F: std::future::Future<Output = T>,
{
    match cancel {
        None => Ok(fut.await),
        Some(flag) => {
            tokio::select! {
                biased;
                _ = wait_chat_cancel(flag) => Err(chat_cancelled_error()),
                result = fut => Ok(result),
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct ChatTurnResult {
    pub session_id: Uuid,
    pub assistant_message: String,
    pub tool_calls: Vec<ToolCallSummary>,
    /// Turn paused waiting for human approval on a mutating tool.
    pub awaiting_approval: bool,
}

#[allow(clippy::too_many_arguments)]
pub async fn resume_chat_after_approval(
    config: &Config,
    store: Arc<dyn Store>,
    github: Arc<GithubHarness>,
    llm: Arc<LlmClient>,
    session_id: Uuid,
    resume: ResumeChatAfterApproval,
    progress: Option<broadcast::Sender<AppEvent>>,
    cancel: Option<Arc<AtomicBool>>,
) -> Result<ChatTurnResult> {
    run_chat_turn(
        config,
        store,
        github,
        llm,
        ChatTurnInput {
            session_id: Some(session_id),
            user_message: String::new(),
            progress,
            cancel,
            resume: Some(resume),
        },
    )
    .await
}

pub async fn run_chat_turn(
    config: &Config,
    store: Arc<dyn Store>,
    github: Arc<GithubHarness>,
    llm: Arc<LlmClient>,
    input: ChatTurnInput,
) -> Result<ChatTurnResult> {
    let ChatTurnInput {
        session_id,
        user_message,
        progress,
        cancel,
        resume,
    } = input;
    let is_resume = resume.is_some();
    let user_message = user_message.as_str();
    if !config.chat.enabled {
        return Err(CoworkerError::Workflow(
            "chat mode disabled in config".into(),
        ));
    }
    if !github.is_available() {
        tracing::warn!(
            "GitHub harness unavailable — coding chat continues; GitHub tools need MCP or bash_run gh"
        );
    }

    let workspace = config.chat.workspace.clone();
    let mut session = match session_id {
        Some(id) => store
            .get_chat_session(&id)
            .await?
            .ok_or_else(|| CoworkerError::Workflow(format!("unknown chat session {id}")))?,
        None if is_resume => {
            return Err(CoworkerError::Workflow(
                "resume chat requires session_id".into(),
            ));
        }
        None => {
            let title = user_message.chars().take(48).collect::<String>();
            store.create_chat_session(Some(&title), None).await?
        }
    };
    let session_id = session.id;

    if !is_resume {
        append_message(
            store.as_ref(),
            &session.id,
            ChatRole::User,
            user_message,
            None,
            None,
        )
        .await?;
    }

    let git_summary = build_workspace_git_summary(&workspace);
    let workspace_display = workspace.to_string_lossy().to_string();
    let history = store
        .list_chat_messages(&session.id, config.chat.history_messages as usize)
        .await?;
    let user_task = if is_resume {
        history
            .iter()
            .rev()
            .find(|m| m.role == ChatRole::User)
            .map(|m| m.content.clone())
            .unwrap_or_default()
    } else {
        user_message.to_string()
    };
    let user_task = user_task.as_str();
    let skill_paths: Vec<_> = config
        .chat
        .skills
        .iter()
        .map(std::path::PathBuf::from)
        .collect();
    let tool_mode = effective_chat_tool_mode(config.chat.tool_mode, github.as_ref());
    let lazy_skills = matches!(tool_mode, ChatToolMode::Auto | ChatToolMode::Lazy);
    let tools_doc = String::new();
    let (prompt_bundle, skill_registry) = load_chat_prompt_bundle_for_session(
        &config.chat.agent,
        &skill_paths,
        tools_doc,
        String::new(),
        user_task,
        lazy_skills,
    )?;
    let loaded_skill_names: Vec<String> = prompt_bundle
        .skills
        .iter()
        .map(|s| s.name.clone())
        .collect();
    let focus_lines =
        build_message_focus_lines(store.as_ref(), user_task, &config.repos).await?;
    let prev_state = if session.runtime_state.revision > 0
        || !session.runtime_state.workspace_path.is_empty()
    {
        Some(&session.runtime_state)
    } else {
        None
    };
    let recent_edits = session.runtime_state.recent_edits.clone();
    let project_agents = load_workspace_agents_md(&workspace);
    let runtime_plan = plan_runtime_context(RuntimeContextInput {
        workspace_path: &workspace_display,
        git_summary: &git_summary,
        recent_edits: &recent_edits,
        loaded_skills: loaded_skill_names,
        focus_lines,
        project_instructions: project_agents.as_deref(),
        prev_state,
    });
    session.runtime_state = runtime_plan.new_state.clone();
    store.update_chat_session(&session).await?;
    let runtime_panel = (runtime_plan.full_body.clone(), runtime_plan.revision);
    let tool_catalog = tool_catalog::ToolCatalog::new();
    let mut discovery_state = ChatDiscoveryState::with_bootstrap(
        user_task,
        skill_registry,
        &prompt_bundle.skills,
    );
    discovery_state.rehydrate_from_tool_history(&history);
    let discovery = Arc::new(Mutex::new(discovery_state));
    if lazy_skills {
        let flow = format_skill_bootstrap_flow(&prompt_bundle.skills);
        if !flow.is_empty() {
            emit_activity_flow(&progress, ActivityFlowKind::Skill, flow);
        }
    }

    let token_budget = TokenBudget::from_config(config.llm.context_limit);
    let history_token_cap = history_token_budget(&token_budget, config.chat.history_tokens);

    let mut system_content = compose_chat_system_prompt(&prompt_bundle, tool_mode);
    trim_system_content(&mut system_content, token_budget.system_budget());

    let mut llm_messages = vec![LlmTurnMessage::new("system", system_content)];
    if !runtime_plan.skip_llm_injection {
        let session_context = format_session_context_message(&runtime_plan.llm_body);
        if !session_context.is_empty() {
            llm_messages.push(LlmTurnMessage::new("user", session_context));
        }
    }

    let compaction = config.chat.compaction.to_strategy();
    llm_messages.extend(
        pack_session_history_with_llm(
            &history,
            config.chat.history_messages as usize,
            history_token_cap,
            llm.as_ref(),
            config.chat.compress_history,
            config.chat.history_summary_min_tokens,
            compaction,
        )
        .await?,
    );
    prune_stale_missing_arg_nudges(&mut llm_messages);

    if let Some(resume) = resume {
        apply_approval_resolution(
            store.as_ref(),
            &session_id,
            &mut session,
            &mut llm_messages,
            &resume,
            &progress,
        )
        .await?;
    }

    let max_turns = config.chat.max_turns;
    let max_tools = config.chat.max_tool_calls;
    let max_duration_secs = config.chat.max_duration_secs;
    let mut tool_calls = Vec::new();
    let mut tools_used = 0u32;
    let mut tool_exec_records = rehydrate_tool_exec_records_from_messages(&history);
    let mut duplicate_tool_nudges: HashMap<String, u32> = HashMap::new();
    let mut duplicate_ui_shown: HashSet<String> = HashSet::new();
    let mut duplicate_forced_reply_nudged = false;
    let mut harness_corrections = 0u32;
    let turn_started = Instant::now();
    let mut llm_rounds = 0u32;
    let config_arc = Arc::new(config.clone());
    let hook_runner = HookRunner::builtin();

    emit_context_snapshot(
        &progress,
        &llm_messages,
        0,
        &token_budget,
        &discovery,
        tool_mode,
        Some((runtime_panel.0.as_str(), runtime_panel.1)),
    )
    .await;

    loop {
        ensure_chat_not_cancelled(&cancel)?;
        if chat_duration_exceeded(max_duration_secs, turn_started) {
            break;
        }
        if chat_limit_reached(max_turns, llm_rounds) {
            break;
        }
        if harness_corrections > MAX_HARNESS_CORRECTIONS {
            break;
        }
        llm_rounds += 1;

        let native_tools = native_tools_for_session(&discovery, tool_mode).await;
        let message_budget = message_budget_for_tools(token_budget.input_budget(), &native_tools);
        let estimated_tokens = crate::agent::context::estimate_messages_tokens(&llm_messages)
            + estimate_tools_tokens(&native_tools);
        let mut turn_ctx = TurnContext {
            token_budget: token_budget.clone(),
            estimated_tokens,
            last_tool: None,
            compaction,
            pending_warm_tools: Vec::new(),
        };
        hook_runner.before_llm_turn(&mut turn_ctx)?;

        emit_progress(
            &progress,
            ChatProgress::TurnThinking {
                turn: llm_rounds,
                elapsed_secs: turn_started.elapsed().as_secs(),
            },
        );
        emit_activity_flow_clear(&progress);
        race_chat_cancel(
            cancel.clone(),
            trim_llm_messages_with_llm(
                &mut llm_messages,
                message_budget,
                llm.as_ref(),
                config.chat.compress_history,
                config.chat.history_summary_min_tokens,
                compaction,
            ),
        )
        .await??;
        emit_context_snapshot(
            &progress,
            &llm_messages,
            llm_rounds,
            &token_budget,
            &discovery,
            tool_mode,
            Some((runtime_panel.0.as_str(), runtime_panel.1)),
        )
        .await;
        tracing::debug!(
            "chat context ~{} msg + ~{} tools tokens (budget {})",
            crate::agent::context::estimate_messages_tokens(&llm_messages),
            estimate_tools_tokens(&native_tools),
            token_budget.input_budget()
        );
        let stream_opts = ChatStepOptions {
            compress_reasoning: config.chat.compress_reasoning,
            cancel: cancel.clone(),
        };
        let outcome = match chat_llm_step_timeout(config.chat.llm_step_timeout_secs) {
            Some(timeout) => {
                match race_chat_cancel(cancel.clone(), async {
                    tokio::time::timeout(
                        timeout,
                        llm.chat_agent_step_with_progress(
                            &llm_messages,
                            &native_tools,
                            stream_opts.clone(),
                            |stream| {
                                if !stream.reasoning.is_empty() {
                                    emit_progress(
                                        &progress,
                                        ChatProgress::ReasoningPartial {
                                            text: stream.reasoning,
                                        },
                                    );
                                }
                                if let Some(label) = stream.tool_pending {
                                    emit_progress(&progress, ChatProgress::ToolPending { label });
                                }
                                if let Some(partial) = stream.reply_partial {
                                    emit_progress(
                                        &progress,
                                        ChatProgress::AssistantPartial { text: partial },
                                    );
                                }
                            },
                        ),
                    )
                    .await
                })
                .await
                {
                    Ok(Ok(Ok(outcome))) => Some(outcome),
                    Ok(Ok(Err(e))) => return Err(e),
                    Ok(Err(_)) => {
                        tracing::warn!(
                            "chat llm step timed out after {}s (round {llm_rounds})",
                            timeout.as_secs()
                        );
                        let nudge = "Your LLM turn timed out (too much internal reasoning). \
                             Call one tool via the native tool API, or reply with a short \
                             natural-language answer. No extended thinking.";
                        if harness_retry_or_stop(
                            &mut harness_corrections,
                            &progress,
                            store.as_ref(),
                            &session.id,
                            nudge,
                            &mut llm_messages,
                        )
                        .await?
                        {
                            break;
                        }
                        None
                    }
                    Err(e) => return Err(e),
                }
            }
            None => Some(
                race_chat_cancel(
                    cancel.clone(),
                    llm.chat_agent_step_with_progress(
                        &llm_messages,
                        &native_tools,
                        stream_opts,
                        |stream| {
                            if !stream.reasoning.is_empty() {
                                emit_progress(
                                    &progress,
                                    ChatProgress::ReasoningPartial {
                                        text: stream.reasoning,
                                    },
                                );
                            }
                            if let Some(label) = stream.tool_pending {
                                emit_progress(&progress, ChatProgress::ToolPending { label });
                            }
                            if let Some(partial) = stream.reply_partial {
                                emit_progress(
                                    &progress,
                                    ChatProgress::AssistantPartial { text: partial },
                                );
                            }
                        },
                    ),
                )
                .await??,
            ),
        };
        let Some(outcome) = outcome else {
            continue;
        };
        if let Some(raw) = &outcome.reasoning_for_context {
            if crate::llm::chat::should_compress_reasoning(
                config.chat.compress_reasoning,
                raw,
                config.chat.reasoning_compress_min_chars,
            ) {
                emit_progress(&progress, ChatProgress::ReasoningCompressing);
            }
            let summary_body = crate::llm::chat::materialize_reasoning_for_context(
                llm.as_ref(),
                raw,
                config.chat.compress_reasoning,
                config.chat.reasoning_compress_min_chars,
            )
            .await?;
            if !summary_body.trim().is_empty() {
                let content = format!("[agent reasoning summary]\n\n{summary_body}");
                llm_messages.push(LlmTurnMessage::new("user", content.clone()));
                persist_reasoning_summary(store.as_ref(), &session.id, &progress, &content).await?;
            }
            emit_context_snapshot(
                &progress,
                &llm_messages,
                llm_rounds,
                &token_budget,
                &discovery,
                tool_mode,
                Some((runtime_panel.0.as_str(), runtime_panel.1)),
            )
            .await;
        }
        let step = outcome.step;
        tracing::debug!("chat llm round {llm_rounds}: {:?}", step.action);
        persist_interim_assistant_message(store.as_ref(), &session.id, &step).await?;

        match step.action {
            ChatAgentAction::Reply => {
                let message = if step.message.trim().is_empty() {
                    "Done.".into()
                } else {
                    step.message
                };
                if crate::agent::context::is_tool_result_transcript(&message) {
                    let nudge = "Your reply must be a natural-language answer for the user, \
not a tool-result transcript. Synthesize from tool results already in context.";
                    if harness_retry_or_stop(
                        &mut harness_corrections,
                        &progress,
                        store.as_ref(),
                        &session.id,
                        nudge,
                        &mut llm_messages,
                    )
                    .await?
                    {
                        break;
                    }
                    continue;
                }
                let tool_names = tool_call_names(&tool_calls);
                if reply_premature_for_task(&message, user_task, &tool_names) {
                    emit_progress(
                        &progress,
                        ChatProgress::TurnThinking {
                            turn: llm_rounds,
                            elapsed_secs: turn_started.elapsed().as_secs(),
                        },
                    );
                    let nudge = reply_premature_nudge(&message, user_task);
                    if harness_retry_or_stop(
                        &mut harness_corrections,
                        &progress,
                        store.as_ref(),
                        &session.id,
                        &nudge,
                        &mut llm_messages,
                    )
                    .await?
                    {
                        break;
                    }
                    continue;
                }
                append_message(
                    store.as_ref(),
                    &session.id,
                    ChatRole::Assistant,
                    &message,
                    None,
                    None,
                )
                .await?;
                append_assistant_to_llm_context(&mut llm_messages, &message);
                emit_context_snapshot(
                    &progress,
                    &llm_messages,
                    llm_rounds,
                    &token_budget,
                    &discovery,
                    tool_mode,
                    Some((runtime_panel.0.as_str(), runtime_panel.1)),
                )
                .await;
                return Ok(ChatTurnResult {
                    session_id: session.id,
                    assistant_message: message,
                    tool_calls,
                    awaiting_approval: false,
                });
            }
            ChatAgentAction::Tool => {
                if chat_limit_reached(max_tools, tools_used) {
                    let nudge = "Tool budget exhausted — reply with your best answer \
from tool results already in context.";
                    if harness_retry_or_stop(
                        &mut harness_corrections,
                        &progress,
                        store.as_ref(),
                        &session.id,
                        nudge,
                        &mut llm_messages,
                    )
                    .await?
                    {
                        break;
                    }
                    continue;
                }

                if step.tool_calls.is_empty() {
                    let nudge = "Tool action missing tool_calls — call at least one tool via the native tool API.";
                    if harness_retry_or_stop(
                        &mut harness_corrections,
                        &progress,
                        store.as_ref(),
                        &session.id,
                        nudge,
                        &mut llm_messages,
                    )
                    .await?
                    {
                        break;
                    }
                    continue;
                }

                let prepared: Vec<PreparedToolCall> = step
                    .tool_calls
                    .iter()
                    .map(|c| prepare_tool_call(c, &config.repos, user_task))
                    .collect();

                if let Some(nudge) = validate_prepared_tool_calls(&prepared, &tool_catalog) {
                    if harness_retry_or_stop(
                        &mut harness_corrections,
                        &progress,
                        store.as_ref(),
                        &session.id,
                        &nudge,
                        &mut llm_messages,
                    )
                    .await?
                    {
                        break;
                    }
                    continue;
                }

                let batch_size = prepared.len() as u32;
                if max_tools > 0 && tools_used.saturating_add(batch_size) > max_tools {
                    let nudge = "Tool budget exhausted — reply with your best answer \
from tool results already in context.";
                    if harness_retry_or_stop(
                        &mut harness_corrections,
                        &progress,
                        store.as_ref(),
                        &session.id,
                        nudge,
                        &mut llm_messages,
                    )
                    .await?
                    {
                        break;
                    }
                    continue;
                }

                if let Some(duplicate) = prepared.iter().find_map(|call| {
                    duplicate_tool_block_reason(tool_exec_records.get(&call.fingerprint))
                        .map(|block| (call.clone(), block))
                }) {
                    let (call, block) = duplicate;
                    let mut round = ToolRoundState {
                        harness_corrections: &mut harness_corrections,
                        progress: &progress,
                        store: store.as_ref(),
                        session_id: &session.id,
                        llm_messages: &mut llm_messages,
                        duplicate_tool_nudges: &mut duplicate_tool_nudges,
                        duplicate_ui_shown: &mut duplicate_ui_shown,
                        duplicate_forced_reply_nudged: &mut duplicate_forced_reply_nudged,
                        tool_calls: &mut tool_calls,
                        tool_exec_records: &mut tool_exec_records,
                        tool_catalog: &tool_catalog,
                        configured_repos: &config.repos,
                        user_task,
                        discovery: discovery.clone(),
                    };
                    let auto_fulfill = block == DuplicateToolBlock::AlreadySucceeded
                        && (call.name == "skill_load" || prepared.len() == 1);
                    if auto_fulfill
                        && fulfill_duplicate_readonly_tool(
                            &mut round,
                            &step,
                            &call,
                            &prepared,
                        )
                        .await?
                    {
                        continue;
                    }
                    if maybe_block_duplicate_tool_call(&mut round, &call, block).await? {
                        break;
                    }
                    continue;
                }

                push_native_assistant_tool_calls(&mut llm_messages, &step);

                let (readonly, mutating): (Vec<_>, Vec<_>) = prepared
                    .into_iter()
                    .partition(|call| !is_mutating_tool(&call.name));

                if !readonly.is_empty() {
                    tools_used += readonly.len() as u32;
                    let outcomes = execute_readonly_tools_parallel(
                        store.clone(),
                        github.clone(),
                        discovery.clone(),
                        cancel.clone(),
                        progress.clone(),
                        ReadonlyToolContext {
                            configured_repos: &config.repos,
                            user_task,
                            bash: &config.chat.bash,
                            python: &config.chat.python,
                            workspace: &workspace,
                            llm: Arc::clone(&llm),
                            config: Arc::clone(&config_arc),
                            progress: progress.clone(),
                        },
                        readonly,
                    )
                    .await?;
                    let mut turn_awaiting_approval = false;
                    for outcome in outcomes {
                        if let Some(review) = outcome.llm_review_rejected.clone() {
                            let PreparedToolCall {
                                id,
                                name,
                                args,
                                ..
                            } = &outcome.call;
                            match handle_llm_review_rejection(
                                store.as_ref(),
                                &session_id,
                                &workspace,
                                config,
                                &store,
                                &github,
                                &progress,
                                &mut llm_messages,
                                &mut tool_calls,
                                id,
                                name,
                                args,
                                &review,
                            )
                            .await?
                            {
                                LlmReviewRejectionOutcome::AutoApproved { detail } => {
                                    if file_tools::is_mutating_file_tool(name) {
                                        record_session_file_edit(
                                            &mut session,
                                            name,
                                            args,
                                            &detail,
                                        );
                                        store_update_session_runtime(store.as_ref(), &session)
                                            .await?;
                                    }
                                    let mut round = ToolRoundState {
                                        harness_corrections: &mut harness_corrections,
                                        progress: &progress,
                                        store: store.as_ref(),
                                        session_id: &session.id,
                                        llm_messages: &mut llm_messages,
                                        duplicate_tool_nudges: &mut duplicate_tool_nudges,
                                        duplicate_ui_shown: &mut duplicate_ui_shown,
                                        duplicate_forced_reply_nudged: &mut duplicate_forced_reply_nudged,
                                        tool_calls: &mut tool_calls,
                                        tool_exec_records: &mut tool_exec_records,
                                        tool_catalog: &tool_catalog,
                                        configured_repos: &config.repos,
                                        user_task,
                                        discovery: discovery.clone(),
                                    };
                                    record_tool_outcome(
                                        &mut round,
                                        ReadonlyToolOutcome {
                                            call: outcome.call,
                                            output: detail,
                                            ok: true,
                                            llm_review_rejected: None,
                                        },
                                    )
                                    .await?;
                                }
                                LlmReviewRejectionOutcome::AwaitingApproval => {
                                    turn_awaiting_approval = true;
                                }
                            }
                            continue;
                        }
                        if outcome.ok && file_tools::is_mutating_file_tool(&outcome.call.name) {
                            record_session_file_edit(
                                &mut session,
                                &outcome.call.name,
                                &outcome.call.args,
                                &outcome.output,
                            );
                            store_update_session_runtime(store.as_ref(), &session).await?;
                        }
                        if outcome.ok {
                            hook_runner.after_tool_result(
                                &mut turn_ctx,
                                &outcome.call.name,
                                &outcome.output,
                            )?;
                        }
                        let mut round = ToolRoundState {
                            harness_corrections: &mut harness_corrections,
                            progress: &progress,
                            store: store.as_ref(),
                            session_id: &session.id,
                            llm_messages: &mut llm_messages,
                            duplicate_tool_nudges: &mut duplicate_tool_nudges,
                            duplicate_ui_shown: &mut duplicate_ui_shown,
                            duplicate_forced_reply_nudged: &mut duplicate_forced_reply_nudged,
                            tool_calls: &mut tool_calls,
                            tool_exec_records: &mut tool_exec_records,
                            tool_catalog: &tool_catalog,
                            configured_repos: &config.repos,
                            user_task,
                            discovery: discovery.clone(),
                        };
                        record_tool_outcome(&mut round, outcome).await?;
                    }
                    if turn_awaiting_approval {
                        return Ok(ChatTurnResult {
                            session_id: session.id,
                            assistant_message: String::new(),
                            tool_calls,
                            awaiting_approval: true,
                        });
                    }
                    if !turn_ctx.pending_warm_tools.is_empty() {
                        let mut state = discovery.lock().await;
                        for name in turn_ctx.pending_warm_tools.drain(..) {
                            state.warm_tool(&name);
                        }
                    }
                    emit_context_snapshot(
                        &progress,
                        &llm_messages,
                        llm_rounds,
                        &token_budget,
                        &discovery,
                        tool_mode,
                        Some((runtime_panel.0.as_str(), runtime_panel.1)),
                    )
                    .await;
                }

                if let Some(mut_call) = mutating.first().cloned() {
                    if mutating.len() > 1 {
                        tracing::warn!(
                            "model returned {} mutating tool_calls; only the first runs per round",
                            mutating.len()
                        );
                    }
                    tools_used += 1;
                    match handle_mutating_tool_call(
                        MutatingToolContext {
                            store: store.as_ref(),
                            session_id: &session_id,
                            session: &mut session,
                            workspace: &workspace,
                            step: &step,
                            config,
                            store_arc: &store,
                            github: &github,
                            progress: &progress,
                            llm_messages: &mut llm_messages,
                            tool_calls: &mut tool_calls,
                            llm_rounds,
                            token_budget: &token_budget,
                            discovery: discovery.clone(),
                            tool_mode,
                            runtime_panel: runtime_panel.clone(),
                        },
                        &mut_call,
                    )
                    .await?
                    {
                        MutatingToolOutcome::Continue => {}
                        MutatingToolOutcome::AwaitingApproval => {
                            return Ok(ChatTurnResult {
                                session_id: session.id,
                                assistant_message: String::new(),
                                tool_calls,
                                awaiting_approval: true,
                            });
                        }
                    }
                }
            }
        }
    }

    let stop = if harness_corrections > MAX_HARNESS_CORRECTIONS {
        ChatStopReason::HarnessCorrections {
            max: MAX_HARNESS_CORRECTIONS,
        }
    } else {
        chat_stop_reason(max_duration_secs, max_turns, turn_started)
    };
    let fallback = synthesize_turn_exhausted_reply(&tool_calls, user_task, stop);
    append_message(
        store.as_ref(),
        &session.id,
        ChatRole::Assistant,
        &fallback,
        None,
        None,
    )
    .await?;
    append_assistant_to_llm_context(&mut llm_messages, &fallback);
    emit_context_snapshot(
        &progress,
        &llm_messages,
        llm_rounds,
        &token_budget,
        &discovery,
        tool_mode,
        Some((runtime_panel.0.as_str(), runtime_panel.1)),
    )
    .await;
    Ok(ChatTurnResult {
        session_id: session.id,
        assistant_message: fallback,
        tool_calls,
        awaiting_approval: false,
    })
}

async fn append_message(
    store: &dyn Store,
    session_id: &Uuid,
    role: ChatRole,
    content: &str,
    tool_name: Option<&str>,
    tool_calls_json: Option<String>,
) -> Result<()> {
    store
        .append_chat_message(&ChatMessage {
            id: Uuid::new_v4(),
            session_id: *session_id,
            role,
            content: content.to_string(),
            ts: Utc::now(),
            tool_name: tool_name.map(str::to_string),
            tool_calls_json,
        })
        .await
}

async fn persist_native_assistant_tool_call(
    store: &dyn Store,
    session_id: &Uuid,
    step: &ChatAgentStep,
) -> Result<()> {
    let calls: Vec<LlmToolCall> = step
        .tool_calls
        .iter()
        .map(|call| LlmToolCall {
            id: call.id.clone(),
            name: call.name.clone(),
            arguments: call.args.clone(),
        })
        .collect();
    let tool_calls_json = serde_json::to_string(&calls)?;
    append_message(
        store,
        session_id,
        ChatRole::Assistant,
        &step.message,
        None,
        Some(tool_calls_json),
    )
    .await
}

async fn apply_approval_resolution(
    store: &dyn Store,
    session_id: &Uuid,
    session: &mut crate::store::ChatSession,
    llm_messages: &mut Vec<LlmTurnMessage>,
    resume: &ResumeChatAfterApproval,
    progress: &Option<broadcast::Sender<AppEvent>>,
) -> Result<()> {
    let ResumeChatAfterApproval {
        approval_id,
        approved,
        detail,
        tool_name,
        tool_args,
    } = resume;

    if *approved && file_tools::is_mutating_file_tool(tool_name) {
        record_session_file_edit(session, tool_name, tool_args, detail);
        store_update_session_runtime(store, session).await?;
    }

    let body = if *approved {
        format!("Approved: {detail}")
    } else {
        format!("Approval denied: {detail}")
    };
    let ctx = format_tool_context_message(tool_name, tool_args, *approved, &body);
    let marker = format!("approval_id={approval_id}");
    let history = store.list_chat_messages(session_id, 10_000).await?;
    if let Some(msg) = history
        .iter()
        .rev()
        .find(|m| m.role == ChatRole::Tool && m.content.contains(&marker))
    {
        let mut updated = msg.clone();
        updated.content = ctx.clone();
        updated.ts = Utc::now();
        store.update_chat_message(&updated).await?;
    }

    if let Some(msg) = llm_messages
        .iter_mut()
        .rev()
        .find(|m| m.role == "tool" && m.content.contains(&marker))
    {
        msg.content = ctx.clone();
    } else {
        llm_messages.push(LlmTurnMessage::tool_result(tool_name, ctx));
    }

    emit_progress(
        progress,
        ChatProgress::ApprovalResolved {
            approval_id: *approval_id,
            tool_name: tool_name.clone(),
            approved: *approved,
            detail: detail.clone(),
        },
    );
    if *approved && crate::agent::review_gate::is_review_gated_tool(tool_name) {
        push_harness_nudge(
            llm_messages,
            format!(
                "Tool `{tool_name}` was approved and ran successfully. \
                 Do not call it again for the same task — reply to the user in natural language."
            ),
        );
    }
    Ok(())
}

fn interim_assistant_message(step: &ChatAgentStep) -> Option<String> {
    if step.action != ChatAgentAction::Tool {
        return None;
    }
    let message = step.message.trim();
    if message.is_empty()
        || message.starts_with('{')
        || crate::agent::context::is_tool_result_transcript(message)
    {
        return None;
    }
    if message.len() > 800 {
        return None;
    }
    Some(message.to_string())
}

async fn persist_interim_assistant_message(
    store: &dyn Store,
    session_id: &Uuid,
    step: &ChatAgentStep,
) -> Result<()> {
    let Some(message) = interim_assistant_message(step) else {
        return Ok(());
    };
    append_message(store, session_id, ChatRole::Assistant, &message, None, None).await
}

fn is_mutating_tool(name: &str) -> bool {
    MUTATING_TOOLS.contains(&name)
}

fn autofill_default_repo(configured_repos: &[String], tool_name: &str, tool_args: &mut Value) {
    if configured_repos.len() != 1 || !tool_catalog::tool_accepts_repo(tool_name) {
        return;
    }
    let Some(map) = tool_args.as_object_mut() else {
        return;
    };
    let empty = map
        .get("repo")
        .and_then(|v| v.as_str())
        .is_none_or(|s| s.trim().is_empty());
    if empty {
        map.insert("repo".to_string(), json!(configured_repos[0].as_str()));
    }
}

fn autofill_repo_from_task(user_task: &str, tool_name: &str, tool_args: &mut Value) {
    if !tool_catalog::tool_accepts_repo(tool_name) {
        return;
    }
    let Some(map) = tool_args.as_object_mut() else {
        return;
    };
    let empty = map
        .get("repo")
        .and_then(|v| v.as_str())
        .is_none_or(|s| s.trim().is_empty());
    if !empty {
        return;
    }
    if let Some((repo, _)) = crate::agent::chat_discovery::extract_github_pr_link(user_task) {
        map.insert("repo".to_string(), json!(repo));
    }
}

fn autofill_pr_from_task(user_task: &str, tool_name: &str, tool_args: &mut Value) {
    if !tool_catalog::tool_accepts_pr_number(tool_name) {
        return;
    }
    let Some(map) = tool_args.as_object_mut() else {
        return;
    };
    if map.get("pr_number").is_some() || map.get("pr").is_some() {
        return;
    }
    let lower = user_task.to_ascii_lowercase();
    if let Some(pr) = crate::agent::chat_discovery::extract_pr_number_for_autofill(&lower) {
        map.insert("pr_number".to_string(), json!(pr));
    }
}

/// Normalize/coerce business tool args (direct call or nested under `tool_call`).
fn finalize_tool_args(
    tool_name: &str,
    tool_args: &mut Value,
    configured_repos: &[String],
    user_task: &str,
) {
    if tool_name == "tool_call" {
        if let Some(inner) = tool_args
            .get("name")
            .and_then(|v| v.as_str())
            .map(str::to_string)
        {
            if !tool_args.get("args").map(Value::is_object).unwrap_or(false) {
                tool_args["args"] = json!({});
            }
            if let Some(inner_args) = tool_args.get_mut("args") {
                finalize_tool_args_inner(&inner, inner_args, configured_repos, user_task);
            }
        }
        return;
    }
    finalize_tool_args_inner(tool_name, tool_args, configured_repos, user_task);
}

fn finalize_tool_args_inner(
    tool_name: &str,
    tool_args: &mut Value,
    configured_repos: &[String],
    user_task: &str,
) {
    coerce_numeric_tool_args(tool_name, tool_args);
    normalize_pr_tool_args(tool_name, tool_args);
    fill_default_diff_max_bytes(tool_name, tool_args);
    autofill_repo_from_task(user_task, tool_name, tool_args);
    autofill_default_repo(configured_repos, tool_name, tool_args);
    autofill_pr_from_task(user_task, tool_name, tool_args);
    normalize_pr_tool_args(tool_name, tool_args);
}

fn fill_default_diff_max_bytes(tool_name: &str, tool_args: &mut Value) {
    if tool_name == "pr_get_diff" && tool_args.get("max_bytes").is_none() {
        tool_args["max_bytes"] = json!(48_000);
    }
}

fn tool_arg_u64(args: &Value, key: &str) -> Option<u64> {
    args.get(key).and_then(tool_arg_u64_from_value)
}

fn tool_arg_u64_from_value(v: &Value) -> Option<u64> {
    v.as_u64()
        .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
        .or_else(|| v.as_i64().filter(|n| *n >= 0).map(|n| n as u64))
        .or_else(|| {
            v.as_f64()
                .filter(|n| n.is_finite() && n.fract() == 0.0 && *n >= 0.0)
                .map(|n| n as u64)
        })
}

/// Normalize PR-tool args so fingerprint / fill / fetched tracking agree (`pr` alias, trimmed repo).
fn normalize_pr_tool_args(tool_name: &str, tool_args: &mut Value) {
    let Some(map) = tool_args.as_object_mut() else {
        return;
    };
    if tool_requires_pr_number(tool_name) {
        if map.get("pr_number").is_none() {
            if let Some(n) = map.get("pr").and_then(tool_arg_u64_from_value) {
                map.insert("pr_number".to_string(), json!(n));
            }
        }
        map.remove("pr");
        if let Some(n) = map.get("pr_number").and_then(tool_arg_u64_from_value) {
            map.insert("pr_number".to_string(), json!(n));
        }
    }
    if let Some(Value::String(repo)) = map.get_mut("repo") {
        *repo = sanitize_repo_string(repo);
    }
}

/// Strip display-style prefixes the model sometimes copies (`repo=owner/name`).
pub fn sanitize_repo_string(raw: &str) -> String {
    let mut s = raw.trim().to_string();
    if let Some(rest) = s.strip_prefix("repo=") {
        s = rest.trim().to_string();
    } else if let Some(rest) = s.strip_prefix("repo = ") {
        s = rest.trim().to_string();
    }
    if let Some(slug) = crate::agent::chat_discovery::extract_github_repo_slug(&s) {
        return slug;
    }
    if s.ends_with(".git") {
        s.truncate(s.len() - 4);
    }
    s
}

fn normalized_pr_number(tool_args: &Value) -> Option<u32> {
    tool_arg_u64(tool_args, "pr_number").map(|n| n as u32)
}

fn coerce_numeric_tool_args(tool_name: &str, tool_args: &mut Value) {
    if tool_requires_pr_number(tool_name) {
        if let Some(n) = tool_arg_u64(tool_args, "pr_number") {
            tool_args["pr_number"] = json!(n);
        }
    }
    if matches!(
        tool_name,
        "ci_get_run_summary" | "ci_get_failed_logs" | "ci_rerun_workflow" | "ci_failure_fingerprint"
    ) {
        if let Some(v) = tool_args.get("run_id") {
            if let Some(n) = v.as_i64() {
                tool_args["run_id"] = json!(n);
            } else if let Some(s) = v.as_str().and_then(|s| s.trim().parse::<i64>().ok()) {
                tool_args["run_id"] = json!(s);
            }
        }
    }
    if tool_name == "ci_compare_runs" {
        for key in ["run_id_a", "run_id_b"] {
            if let Some(v) = tool_args.get(key) {
                if let Some(n) = v.as_i64() {
                    tool_args[key] = json!(n);
                } else if let Some(s) = v.as_str().and_then(|s| s.trim().parse::<i64>().ok()) {
                    tool_args[key] = json!(s);
                }
            }
        }
    }
}

fn normalize_model_tool_args(tool_name: &str, tool_args: &mut Value) {
    if tool_name == "tool_call" {
        normalize_meta_tool_call_args(tool_args);
    } else {
        crate::llm::chat::flatten_tool_args(tool_args);
        if tool_name == "write_file" {
            normalize_write_file_args(tool_args);
        }
    }
}

/// Models sometimes pass edit_file's `new_string` to write_file.
fn normalize_write_file_args(tool_args: &mut Value) {
    let Some(map) = tool_args.as_object_mut() else {
        return;
    };
    let has_content = map
        .get("content")
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.trim().is_empty());
    if has_content {
        return;
    }
    if let Some(ns) = map
        .get("new_string")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
    {
        map.insert("content".to_string(), Value::String(ns.to_string()));
    }
}

fn parse_stringified_json_arg(value: &mut Value) {
    while let Some(s) = value.as_str() {
        let trimmed = s.trim();
        let Ok(parsed) = serde_json::from_str::<Value>(trimmed) else {
            break;
        };
        *value = parsed;
    }
}

fn normalize_meta_tool_call_args(tool_args: &mut Value) {
    parse_stringified_json_arg(tool_args);
    let Some(map) = tool_args.as_object_mut() else {
        return;
    };

    let mut target_args = serde_json::Map::new();
    for key in ["params", "parameters"] {
        if let Some(mut value) = map.remove(key) {
            parse_stringified_json_arg(&mut value);
            if let Some(src) = value.as_object() {
                for (k, v) in src {
                    target_args.entry(k.clone()).or_insert_with(|| v.clone());
                }
            }
        }
    }

    let loose_keys: Vec<String> = map
        .keys()
        .filter(|key| !matches!(key.as_str(), "name" | "args"))
        .cloned()
        .collect();
    for key in loose_keys {
        if let Some(value) = map.remove(&key) {
            if !value.is_null() {
                target_args.entry(key).or_insert(value);
            }
        }
    }

    if let Some(args) = map.get_mut("args") {
        parse_stringified_json_arg(args);
    }
    if !target_args.is_empty() {
        let args = map
            .entry("args".to_string())
            .or_insert_with(|| Value::Object(serde_json::Map::new()));
        if let Some(dst) = args.as_object_mut() {
            for (key, value) in target_args {
                dst.entry(key).or_insert(value);
            }
        }
    }
}

fn chat_limit_reached(max: u32, used: u32) -> bool {
    max > 0 && used >= max
}

fn chat_duration_exceeded(max_secs: u64, started: Instant) -> bool {
    max_secs > 0 && started.elapsed().as_secs() >= max_secs
}

fn chat_llm_step_timeout(max_secs: u64) -> Option<Duration> {
    if max_secs == 0 {
        None
    } else {
        Some(Duration::from_secs(max_secs))
    }
}

#[derive(Debug, Clone, Copy)]
enum ChatStopReason {
    Duration { secs: u64 },
    LlmSteps { max: u32 },
    HarnessCorrections { max: u32 },
}

fn chat_stop_reason(max_duration_secs: u64, max_turns: u32, started: Instant) -> ChatStopReason {
    if chat_duration_exceeded(max_duration_secs, started) {
        ChatStopReason::Duration {
            secs: max_duration_secs,
        }
    } else {
        ChatStopReason::LlmSteps { max: max_turns }
    }
}

fn synthesize_turn_exhausted_reply(
    tool_calls: &[ToolCallSummary],
    user_message: &str,
    reason: ChatStopReason,
) -> String {
    let header = match reason {
        ChatStopReason::Duration { secs } => {
            format!("Reached the {secs}s time limit while working on: \"{user_message}\"")
        }
        ChatStopReason::LlmSteps { max } => {
            format!("Reached the {max} LLM step limit while working on: \"{user_message}\"")
        }
        ChatStopReason::HarnessCorrections { max } => format!(
            "Stopped after {max} harness corrections — the model could not produce a valid tool call for: \"{user_message}\""
        ),
    };
    if tool_calls.is_empty() {
        return format!(
            "{header}. Try a narrower question or raise chat.max_duration_secs / chat.max_turns in config."
        );
    }
    let mut parts = vec![header];
    for tc in tool_calls {
        parts.push(String::new());
        parts.push(format!("**{}**", tc.tool_name));
        parts.push(tc.preview(800));
    }
    parts.join("\n")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ToolExecRecord {
    succeeded: bool,
    fail_count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DuplicateToolBlock {
    AlreadySucceeded,
    FailedTooMany,
}

fn duplicate_tool_block_reason(record: Option<&ToolExecRecord>) -> Option<DuplicateToolBlock> {
    let record = record?;
    if record.succeeded {
        return Some(DuplicateToolBlock::AlreadySucceeded);
    }
    if record.fail_count >= 2 {
        return Some(DuplicateToolBlock::FailedTooMany);
    }
    None
}

/// MCP may return tool errors as plain text; treat them as failures for dedup / retry.
///
/// Only inspects the header / first line. Large payloads such as `pr_get_diff` often
/// contain error-like substrings inside added lines in the unified diff.
fn tool_output_indicates_failure(tool_name: &str, output: &str) -> bool {
    if tool_name == "pr_get_diff"
        && crate::agent::context::pr_get_diff_raw_output_is_success(output)
    {
        return false;
    }
    if tool_name == bash_tool::BASH_RUN_TOOL && bash_tool::output_indicates_failure(output) {
        return true;
    }
    if tool_name == python_tool::PYTHON_RUN_TOOL && python_tool::output_indicates_failure(output) {
        return true;
    }
    crate::agent::context::tool_body_header_indicates_failure(output)
}

fn tool_requires_pr_number(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "pr_get_overview"
            | "pr_get_status"
            | "pr_get_merge_blockers"
            | "pr_list_changed_files"
            | "pr_get_diff"
            | "ci_analyze_pr_failures"
    )
}

fn tool_call_names(tool_calls: &[ToolCallSummary]) -> Vec<&str> {
    tool_calls.iter().map(|tc| tc.tool_name.as_str()).collect()
}

fn tool_call_fingerprint(tool_name: &str, tool_args: &Value) -> String {
    if let Some(semantic) = semantic_tool_fingerprint(tool_name, tool_args) {
        return semantic;
    }
    format!("{tool_name}:{}", canonical_tool_args(tool_args))
}

fn semantic_tool_fingerprint(tool_name: &str, tool_args: &Value) -> Option<String> {
    if tool_name == python_tool::PYTHON_RUN_TOOL {
        let code = tool_args
            .get("code")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())?;
        return Some(format!(
            "python_run:code={}",
            normalize_python_fingerprint(code)
        ));
    }
    if tool_name == bash_tool::BASH_RUN_TOOL {
        let command = tool_args
            .get("command")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())?;
        return Some(format!(
            "bash_run:command={}",
            normalize_bash_fingerprint(command)
        ));
    }
    if !tool_requires_pr_number(tool_name) {
        return None;
    }
    let repo = tool_args
        .get("repo")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())?;
    let pr = normalized_pr_number(tool_args)?;
    Some(format!("{tool_name}:repo={repo},pr_number={pr}"))
}

fn canonical_tool_args(value: &Value) -> String {
    let Some(map) = value.as_object() else {
        return value.to_string();
    };
    let mut keys: Vec<&str> = map.keys().map(String::as_str).collect();
    keys.sort_unstable();
    keys.into_iter()
        .filter_map(|key| {
            let val = &map[key];
            if val.is_null() {
                return None;
            }
            Some(format!("{key}={}", canonical_arg_value(val)))
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn canonical_arg_value(value: &Value) -> String {
    match value {
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.trim().to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Array(items) => items
            .iter()
            .map(canonical_arg_value)
            .collect::<Vec<_>>()
            .join("|"),
        Value::Object(_) => canonical_tool_args(value),
        _ => value.to_string(),
    }
}

fn normalize_python_fingerprint(code: &str) -> String {
    code.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn normalize_bash_fingerprint(command: &str) -> String {
    command
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn rehydrate_tool_output_succeeded(tool_name: &str, content: &str) -> bool {
    if content.contains("Approval denied") || content.starts_with("tool_error(") {
        return false;
    }
    let ok_header = content.starts_with("tool_result(")
        || content.contains("Approved:")
        || content.contains("Auto-approved");
    if !ok_header {
        return false;
    }
    if (tool_name == python_tool::PYTHON_RUN_TOOL || tool_name == bash_tool::BASH_RUN_TOOL)
        && content.contains("review: APPROVE")
        && (content.contains("exit: 0") || content.contains("Approved:"))
    {
        return true;
    }
    !tool_output_indicates_failure(tool_name, content)
}

fn rehydrate_tool_exec_records_from_messages(
    messages: &[ChatMessage],
) -> HashMap<String, ToolExecRecord> {
    let mut records = HashMap::new();
    for msg in messages {
        if msg.role != ChatRole::Tool {
            continue;
        }
        let tool_name = match msg.tool_name.as_deref() {
            Some(name) => name,
            None => continue,
        };
        let args_json = match msg.tool_calls_json.as_deref() {
            Some(json) => json,
            None => continue,
        };
        let args: Value = match serde_json::from_str(args_json) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let fingerprint = tool_call_fingerprint(tool_name, &args);
        if msg.content.contains("Approval denied") || msg.content.starts_with("tool_error(") {
            let entry = records.entry(fingerprint).or_insert(ToolExecRecord {
                succeeded: false,
                fail_count: 0,
            });
            entry.fail_count = entry.fail_count.saturating_add(1);
            continue;
        }
        if rehydrate_tool_output_succeeded(tool_name, &msg.content) {
            records.insert(
                fingerprint,
                ToolExecRecord {
                    succeeded: true,
                    fail_count: 0,
                },
            );
        } else if msg.content.starts_with("tool_result(")
            || msg.content.contains("tool_approval_pending(")
        {
            let entry = records.entry(fingerprint).or_insert(ToolExecRecord {
                succeeded: false,
                fail_count: 0,
            });
            entry.fail_count = entry.fail_count.saturating_add(1);
        }
    }
    records
}

fn forced_reply_after_duplicate_tools_nudge(
    user_message: &str,
    tool_calls: &[ToolCallSummary],
) -> String {
    if !tool_calls.is_empty() {
        return format!(
            "Same tool call repeated several times. User asked: \"{user_message}\"\n\
             Reply with an answer from tool results already in context."
        );
    }
    format!(
        "Same tool call repeated several times. User asked: \"{user_message}\"\n\
         Reply with what you have, or explain what is still missing."
    )
}

fn duplicate_tool_nudge(tool_name: &str, block: DuplicateToolBlock) -> String {
    match block {
        DuplicateToolBlock::AlreadySucceeded => format!(
            "Identical `{tool_name}` with the same args was already fetched in this turn. \
             Use those results, call a different tool, or reply."
        ),
        DuplicateToolBlock::FailedTooMany => format!(
            "`{tool_name}` with the same args failed twice in this turn. \
             Reply with what you have, or try different args."
        ),
    }
}

fn ci_analyze_lacks_runs(output: &str) -> bool {
    let lower = output.to_ascii_lowercase();
    if lower.contains("no failing") {
        return true;
    }
    if parse_failing_runs(output).is_empty()
        && (lower.contains("pending") || lower.contains("0 failing run"))
    {
        return true;
    }
    false
}

fn maybe_push_tool_failure_harness_nudge(
    catalog: &tool_catalog::ToolCatalog,
    tool_name: &str,
    tool_args: &Value,
    body: &str,
    configured_repos: &[String],
    messages: &mut Vec<LlmTurnMessage>,
) -> String {
    let (effective_name, effective_args) = effective_tool_for_nudge(tool_name, tool_args);
    let parsed_missing: Vec<String> = missing_params_from_tool_error(body)
        .into_iter()
        .filter(|field| {
            !tool_catalog::ToolCatalog::tool_arg_field_satisfied(effective_args, field)
        })
        .collect();
    let schema_missing = catalog.missing_required_fields(effective_name, effective_args);
    let example_repo = configured_repos.first().map(String::as_str);
    let nudge = if tool_name == "tool_call" && body.contains("JSON object") {
        format!(
            "Tool `tool_call` requires `args` as a JSON object, not a string. \
Example: {{\"name\":\"pr_get_overview\",\"args\":{{\"repo\":\"{}\",\"pr_number\":1}}}}",
            example_repo.unwrap_or("owner/repo")
        )
    } else if let Some(field) = parsed_missing.first() {
        catalog.format_tool_args_nudge(effective_name, field, None, example_repo)
    } else if let Some(field) = schema_missing.first() {
        catalog.format_tool_args_nudge(effective_name, field, None, example_repo)
    } else {
        catalog.format_tool_failure_nudge(effective_name, effective_args, body, configured_repos)
    };
    push_harness_nudge(messages, nudge)
}

fn effective_tool_for_nudge<'a>(tool_name: &'a str, tool_args: &'a Value) -> (&'a str, &'a Value) {
    if tool_name == "tool_call" {
        if let Some(inner) = tool_args.get("name").and_then(|v| v.as_str()) {
            let args = tool_args.get("args").unwrap_or(tool_args);
            return (inner, args);
        }
    }
    (tool_name, tool_args)
}

fn missing_params_from_tool_error(body: &str) -> Vec<String> {
    let marker = "missing required parameter(s):";
    let Some(idx) = body.find(marker) else {
        return Vec::new();
    };
    let rest = body[idx + marker.len()..].trim();
    let end = rest.find('.').unwrap_or(rest.len());
    rest[..end]
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

fn push_harness_nudge(messages: &mut Vec<LlmTurnMessage>, content: String) -> String {
    let base = content.clone();
    let mut retry = 1u32;
    let mut existing_idx = None;
    for (idx, m) in messages.iter().enumerate() {
        if m.role == "user"
            && crate::agent::context::is_harness_nudge_content(&m.content)
            && harness_nudge_base(&m.content) == base
        {
            retry += 1;
            existing_idx = Some(idx);
        }
    }
    let body = if retry > 1 {
        format!(
            "{content}\n\n\
             (Harness retry {retry} — call the tool above via the native tool API; no further reasoning.)"
        )
    } else {
        content
    };
    if let Some(idx) = existing_idx {
        messages[idx].content = body.clone();
    } else {
        messages.push(LlmTurnMessage::new("user", body.clone()));
    }
    body
}

fn missing_arg_nudge_tool_and_field(content: &str) -> Option<(&str, &str)> {
    let base = harness_nudge_base(content).trim_start();
    let rest = base.strip_prefix("Tool `")?;
    let (tool_name, rest) = rest.split_once("` is missing required `")?;
    let (field, _) = rest.split_once('`')?;
    Some((tool_name, field))
}

fn tool_args_satisfy_missing_field(tool_args: &Value, field: &str) -> bool {
    tool_catalog::ToolCatalog::tool_arg_field_satisfied(tool_args, field)
}

fn remove_satisfied_missing_arg_nudges(
    messages: &mut Vec<LlmTurnMessage>,
    tool_name: &str,
    tool_args: &Value,
) {
    messages.retain(|m| {
        if m.role != "user" {
            return true;
        }
        let Some((nudge_tool, field)) = missing_arg_nudge_tool_and_field(&m.content) else {
            return true;
        };
        nudge_tool != tool_name || !tool_args_satisfy_missing_field(tool_args, field)
    });
}

fn is_successful_tool_result_for_message(m: &LlmTurnMessage, tool_name: &str) -> bool {
    if m.role == "tool" {
        return m.tool_name.as_deref() == Some(tool_name)
            && !m.content.trim_start().starts_with("tool_error(");
    }
    is_successful_tool_result_for(&m.content, tool_name)
}

fn is_successful_tool_result_for(content: &str, tool_name: &str) -> bool {
    let trimmed = content.trim_start();
    trimmed.starts_with(&format!("tool_result({tool_name}"))
        || trimmed.starts_with(&format!("[tool_result {tool_name}]"))
        || trimmed.starts_with(&format!("[summarized tool_result {tool_name}]"))
}

fn prune_stale_missing_arg_nudges(messages: &mut Vec<LlmTurnMessage>) {
    let mut stale = HashSet::new();
    for (idx, msg) in messages.iter().enumerate() {
        if msg.role != "user" {
            continue;
        }
        let Some((tool_name, _)) = missing_arg_nudge_tool_and_field(&msg.content) else {
            continue;
        };
        if messages
            .iter()
            .skip(idx + 1)
            .any(|later| is_successful_tool_result_for_message(later, tool_name))
        {
            stale.insert(idx);
        }
    }
    if stale.is_empty() {
        return;
    }
    let mut idx = 0usize;
    messages.retain(|_| {
        let keep = !stale.contains(&idx);
        idx += 1;
        keep
    });
}

async fn persist_reasoning_summary(
    store: &dyn Store,
    session_id: &Uuid,
    progress: &Option<broadcast::Sender<AppEvent>>,
    content: &str,
) -> Result<()> {
    append_message(store, session_id, ChatRole::Reasoning, content, None, None).await?;
    let body = crate::agent::context::strip_reasoning_summary_marker(content);
    let preview = body
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or(body);
    emit_progress(
        progress,
        ChatProgress::ReasoningSummary {
            preview: crate::agent::context::truncate_chars(preview, 120),
            body: body.to_string(),
        },
    );
    Ok(())
}

async fn persist_harness_nudge(
    store: &dyn Store,
    session_id: &Uuid,
    llm_messages: &mut Vec<LlmTurnMessage>,
    nudge: &str,
) -> Result<()> {
    let body = push_harness_nudge(llm_messages, nudge.to_string());
    append_message(store, session_id, ChatRole::Harness, &body, None, None).await
}

/// Push a harness correction to LLM context + session store; return true when the turn should stop.
async fn harness_retry_or_stop(
    harness_corrections: &mut u32,
    progress: &Option<broadcast::Sender<AppEvent>>,
    store: &dyn Store,
    session_id: &Uuid,
    nudge: &str,
    llm_messages: &mut Vec<LlmTurnMessage>,
) -> Result<bool> {
    persist_harness_nudge(store, session_id, llm_messages, nudge).await?;
    *harness_corrections += 1;
    emit_progress(
        progress,
        ChatProgress::HarnessNudge {
            retry: *harness_corrections,
            preview: crate::agent::context::truncate_chars(
                nudge.lines().next().unwrap_or(nudge),
                120,
            ),
        },
    );
    Ok(*harness_corrections > MAX_HARNESS_CORRECTIONS)
}

#[derive(Debug, Clone)]
struct PreparedToolCall {
    id: String,
    name: String,
    args: Value,
    fingerprint: String,
}

struct ToolRoundState<'a> {
    harness_corrections: &'a mut u32,
    progress: &'a Option<broadcast::Sender<AppEvent>>,
    store: &'a dyn Store,
    session_id: &'a Uuid,
    llm_messages: &'a mut Vec<LlmTurnMessage>,
    duplicate_tool_nudges: &'a mut HashMap<String, u32>,
    duplicate_ui_shown: &'a mut HashSet<String>,
    duplicate_forced_reply_nudged: &'a mut bool,
    tool_calls: &'a mut Vec<ToolCallSummary>,
    tool_exec_records: &'a mut HashMap<String, ToolExecRecord>,
    tool_catalog: &'a crate::agent::tool_catalog::ToolCatalog,
    configured_repos: &'a [String],
    user_task: &'a str,
    discovery: Arc<Mutex<ChatDiscoveryState>>,
}

#[derive(Debug, Clone)]
struct ReadonlyToolOutcome {
    call: PreparedToolCall,
    output: String,
    ok: bool,
    llm_review_rejected: Option<crate::agent::bash_tool::BashCommandReview>,
}

enum MutatingToolOutcome {
    Continue,
    AwaitingApproval,
}

fn prepare_tool_call(
    call: &ResolvedToolCall,
    configured_repos: &[String],
    user_task: &str,
) -> PreparedToolCall {
    let mut args = call.args.clone();
    normalize_model_tool_args(&call.name, &mut args);
    finalize_tool_args(&call.name, &mut args, configured_repos, user_task);
    let fingerprint = tool_call_fingerprint(&call.name, &args);
    PreparedToolCall {
        id: call.id.clone(),
        name: call.name.clone(),
        args,
        fingerprint,
    }
}

fn validate_prepared_tool_calls(
    calls: &[PreparedToolCall],
    tool_catalog: &crate::agent::tool_catalog::ToolCatalog,
) -> Option<String> {
    for call in calls {
        if !crate::llm::chat::is_plausible_tool_name(&call.name) {
            return Some(tool_catalog.format_invalid_tool_nudge(&call.name));
        }
        if !tool_catalog.is_known_chat_tool(&call.name) {
            return Some(tool_catalog.format_unknown_tool_nudge(&call.name));
        }
    }
    None
}

/// Idempotent readonly tools: replay cached output instead of harness-nudging the model.
async fn fulfill_duplicate_readonly_tool(
    round: &mut ToolRoundState<'_>,
    step: &ChatAgentStep,
    call: &PreparedToolCall,
    all_calls: &[PreparedToolCall],
) -> Result<bool> {
    if is_mutating_tool(&call.name) {
        return Ok(false);
    }
    let cached = match cached_duplicate_readonly_body(round, call).await {
        Some(cached) => cached,
        None => return Ok(false),
    };
    push_native_assistant_tool_calls(round.llm_messages, step);
    for prep in all_calls {
        if prep.id != call.id {
            continue;
        }
        let ctx = match &cached {
            CachedToolOutput::Transcript(t) => t.clone(),
            CachedToolOutput::Body(body) => {
                format_tool_context_message(&prep.name, &prep.args, true, body)
            }
        };
        round.tool_calls.push(ToolCallSummary {
            tool_name: prep.name.clone(),
            output: ctx.clone(),
        });
        append_message(
            round.store,
            round.session_id,
            ChatRole::Tool,
            &ctx,
            Some(&prep.name),
            Some(prep.args.to_string()),
        )
        .await?;
        round.llm_messages.push(LlmTurnMessage::tool_result_with_id(
            Some(prep.id.clone()),
            prep.name.clone(),
            ctx,
        ));
    }
    tracing::info!(
        "duplicate {}({}) — replayed cached output (no harness nudge)",
        call.name,
        format_tool_args_short(&call.args)
    );
    Ok(true)
}

#[derive(Debug, Clone)]
enum CachedToolOutput {
    /// Already formatted `tool_result(...)` transcript from session context.
    Transcript(String),
    /// Raw tool body to wrap in a new transcript.
    Body(String),
}

async fn cached_duplicate_readonly_body(
    round: &ToolRoundState<'_>,
    call: &PreparedToolCall,
) -> Option<CachedToolOutput> {
    if let Some(prior) = find_prior_tool_result_body(round.llm_messages, &call.name, &call.args) {
        return Some(CachedToolOutput::Transcript(prior));
    }
    if call.name == "skill_load" {
        let name = call.args.get("name").and_then(|v| v.as_str())?;
        let state = round.discovery.lock().await;
        let skill = state.skill_registry.get(name)?.clone();
        return Some(CachedToolOutput::Body(format!(
            "(already loaded — proceed with the skill workflow)\n\n{}",
            SkillRegistry::format_skill_load(&skill)
        )));
    }
    None
}

fn find_prior_tool_result_body(
    messages: &[LlmTurnMessage],
    tool_name: &str,
    args: &Value,
) -> Option<String> {
    let want = canonical_tool_args(args);
    for msg in messages.iter().rev() {
        if msg.role != "tool" || msg.tool_name.as_deref() != Some(tool_name) {
            continue;
        }
        if tool_transcript_matches_args(&msg.content, args, &want) {
            return Some(msg.content.clone());
        }
    }
    None
}

fn tool_transcript_matches_args(content: &str, args: &Value, want_fp: &str) -> bool {
    if let Some(args_line) = content
        .lines()
        .find(|line| line.trim_start().starts_with("args:"))
    {
        let json_part = args_line.trim_start().strip_prefix("args:").unwrap_or("").trim();
        if let Ok(parsed) = serde_json::from_str::<Value>(json_part) {
            if canonical_tool_args(&parsed) == want_fp {
                return true;
            }
        }
    }
    content.contains(&args.to_string()) || canonical_tool_args(args) == want_fp
}

async fn maybe_block_duplicate_tool_call(
    round: &mut ToolRoundState<'_>,
    call: &PreparedToolCall,
    block: DuplicateToolBlock,
) -> Result<bool> {
    if block == DuplicateToolBlock::AlreadySucceeded
        && crate::agent::review_gate::is_review_gated_tool(&call.name)
    {
        if !*round.duplicate_forced_reply_nudged {
            *round.duplicate_forced_reply_nudged = true;
        }
        let nudge = forced_reply_after_duplicate_tools_nudge(round.user_task, round.tool_calls);
        return harness_retry_or_stop(
            round.harness_corrections,
            round.progress,
            round.store,
            round.session_id,
            &nudge,
            round.llm_messages,
        )
        .await;
    }
    let nudge_count = round
        .duplicate_tool_nudges
        .entry(call.fingerprint.clone())
        .or_insert(0);
    *nudge_count += 1;
    if round.duplicate_ui_shown.insert(call.fingerprint.clone()) {
        emit_progress(
            round.progress,
            ChatProgress::DuplicateToolBlocked {
                tool_name: call.name.clone(),
                args_short: format_tool_args_short(&call.args),
                attempt: *nudge_count,
            },
        );
    }
    if *nudge_count >= 2 {
        if !*round.duplicate_forced_reply_nudged {
            *round.duplicate_forced_reply_nudged = true;
            round.duplicate_tool_nudges.remove(&call.fingerprint);
        }
        let nudge = forced_reply_after_duplicate_tools_nudge(round.user_task, round.tool_calls);
        return harness_retry_or_stop(
            round.harness_corrections,
            round.progress,
            round.store,
            round.session_id,
            &nudge,
            round.llm_messages,
        )
        .await;
    }
    let nudge = duplicate_tool_nudge(&call.name, block);
    harness_retry_or_stop(
        round.harness_corrections,
        round.progress,
        round.store,
        round.session_id,
        &nudge,
        round.llm_messages,
    )
    .await
}

fn push_native_assistant_tool_calls(messages: &mut Vec<LlmTurnMessage>, step: &ChatAgentStep) {
    let calls: Vec<LlmToolCall> = step
        .tool_calls
        .iter()
        .map(|call| LlmToolCall {
            id: call.id.clone(),
            name: call.name.clone(),
            arguments: call.args.clone(),
        })
        .collect();
    messages.push(LlmTurnMessage::assistant_tool_call(
        step.message.clone(),
        calls,
    ));
}

struct ReadonlyToolContext<'a> {
    configured_repos: &'a [String],
    user_task: &'a str,
    bash: &'a BashToolConfig,
    python: &'a PythonToolConfig,
    workspace: &'a std::path::Path,
    llm: Arc<LlmClient>,
    config: Arc<Config>,
    progress: Option<broadcast::Sender<AppEvent>>,
}

async fn execute_readonly_tools_parallel(
    store: Arc<dyn Store>,
    github: Arc<GithubHarness>,
    discovery: Arc<Mutex<ChatDiscoveryState>>,
    cancel: Option<Arc<AtomicBool>>,
    progress: Option<broadcast::Sender<AppEvent>>,
    ctx: ReadonlyToolContext<'_>,
    calls: Vec<PreparedToolCall>,
) -> Result<Vec<ReadonlyToolOutcome>> {
    let futures = calls.into_iter().map(|call| {
        let store = Arc::clone(&store);
        let github = Arc::clone(&github);
        let discovery = Arc::clone(&discovery);
        let cancel = cancel.clone();
        let progress = progress.clone();
        let configured_repos = ctx.configured_repos.to_vec();
        let user_task = ctx.user_task.to_string();
        let bash = ctx.bash.clone();
        let python = ctx.python.clone();
        let workspace = ctx.workspace.to_path_buf();
        let llm = Arc::clone(&ctx.llm);
        let config = Arc::clone(&ctx.config);
        let progress_ctx = ctx.progress.clone();
        async move {
            ensure_chat_not_cancelled(&cancel)?;
            let args_short = format_tool_args_short(&call.args);
            let flow_tool = is_flow_activity_tool(&call.name);
            if flow_tool {
                emit_activity_flow(
                    &progress,
                    activity_flow_kind_for_tool(&call.name),
                    format_flow_tool_start(&call.name, &call.args),
                );
            } else {
                emit_progress(
                    &progress,
                    ChatProgress::ToolStart {
                        name: call.name.clone(),
                        args_short,
                    },
                );
            }
            let tool_start = Instant::now();
            let result = match race_chat_cancel(
                cancel.clone(),
                execute_readonly_tool_with_heartbeat(
                    store,
                    github,
                    &discovery,
                    &progress,
                    ReadonlyToolContext {
                        configured_repos: &configured_repos,
                        user_task: &user_task,
                        bash: &bash,
                        python: &python,
                        workspace: &workspace,
                        llm,
                        config,
                        progress: progress_ctx,
                    },
                    &call.name,
                    call.args.clone(),
                ),
            )
            .await
            {
                Ok(r) => r,
                Err(e) => return Err(e),
            };
            let (output, ok, llm_review_rejected) = match result {
                Ok(ReadonlyToolExecuteResult::Output(o))
                    if tool_output_indicates_failure(&call.name, &o) =>
                {
                    (o, false, None)
                }
                Ok(ReadonlyToolExecuteResult::Output(o)) => (o, true, None),
                Ok(ReadonlyToolExecuteResult::LlmReviewRejected(review)) => {
                    (String::new(), false, Some(review))
                }
                Err(e) => (format!("tool error: {e}"), false, None),
            };
            let elapsed_ms = tool_start.elapsed().as_millis();
            let ctx = format_tool_context_message(&call.name, &call.args, ok, &output);
            if flow_tool {
                emit_activity_flow(
                    &progress,
                    activity_flow_kind_for_tool(&call.name),
                    format_flow_tool_done(&call.name, &call.args, ok, &ctx),
                );
                emit_activity_flow_clear(&progress);
            } else {
                emit_progress(
                    &progress,
                    ChatProgress::ToolDone {
                        name: call.name.clone(),
                        args_short: format_tool_args_short(&call.args),
                        ok,
                        elapsed_ms,
                        output_preview: crate::agent::context::truncate_chars(&ctx, 6_000),
                    },
                );
            }
            Ok(ReadonlyToolOutcome {
                call,
                output,
                ok,
                llm_review_rejected,
            })
        }
    });
    join_all(futures).await.into_iter().collect()
}

async fn record_tool_outcome(
    round: &mut ToolRoundState<'_>,
    outcome: ReadonlyToolOutcome,
) -> Result<()> {
    let PreparedToolCall {
        id,
        name,
        args,
        fingerprint,
    } = outcome.call;
    let output = outcome.output;
    let ok = outcome.ok;
    let ctx = format_tool_context_message(&name, &args, ok, &output);
    round.tool_calls.push(ToolCallSummary {
        tool_name: name.clone(),
        output: ctx.clone(),
    });
    let record = round
        .tool_exec_records
        .entry(fingerprint.clone())
        .or_insert(ToolExecRecord {
            succeeded: false,
            fail_count: 0,
        });
    if ok {
        record.succeeded = true;
        round.duplicate_tool_nudges.remove(&fingerprint);
        round.duplicate_ui_shown.remove(&fingerprint);
        let mut state = round.discovery.lock().await;
        state.warm_from_tool_call_args(&name, &args);
    } else {
        record.fail_count += 1;
    }
    append_message(
        round.store,
        round.session_id,
        ChatRole::Tool,
        &ctx,
        Some(&name),
        Some(args.to_string()),
    )
    .await?;
    round.llm_messages.push(LlmTurnMessage::tool_result_with_id(
        Some(id),
        name.clone(),
        ctx.clone(),
    ));
    if ok {
        remove_satisfied_missing_arg_nudges(round.llm_messages, &name, &args);
        prune_stale_missing_arg_nudges(round.llm_messages);
    } else {
        let nudge = maybe_push_tool_failure_harness_nudge(
            round.tool_catalog,
            &name,
            &args,
            &output,
            round.configured_repos,
            round.llm_messages,
        );
        append_message(
            round.store,
            round.session_id,
            ChatRole::Harness,
            &nudge,
            None,
            None,
        )
        .await?;
    }
    if ok && name == "ci_analyze_pr_failures" && ci_analyze_lacks_runs(&output) {
        round.llm_messages.push(LlmTurnMessage::new(
            "user",
            "ci_analyze returned no actionable run IDs in this response \
(pending checks or empty output).",
        ));
    }
    Ok(())
}

struct MutatingToolContext<'a> {
    store: &'a dyn Store,
    session_id: &'a Uuid,
    session: &'a mut crate::store::ChatSession,
    workspace: &'a std::path::Path,
    step: &'a ChatAgentStep,
    config: &'a Config,
    store_arc: &'a Arc<dyn Store>,
    github: &'a Arc<GithubHarness>,
    progress: &'a Option<broadcast::Sender<AppEvent>>,
    llm_messages: &'a mut Vec<LlmTurnMessage>,
    tool_calls: &'a mut Vec<ToolCallSummary>,
    llm_rounds: u32,
    token_budget: &'a TokenBudget,
    discovery: Arc<Mutex<ChatDiscoveryState>>,
    tool_mode: ChatToolMode,
    runtime_panel: (String, u64),
}

async fn handle_mutating_tool_call(
    ctx: MutatingToolContext<'_>,
    call: &PreparedToolCall,
) -> Result<MutatingToolOutcome> {
    let tool_name = call.name.as_str();
    let tool_args = &call.args;
    let queued =
        queue_mutating_approval(ctx.store, ctx.workspace, tool_name, tool_args).await?;
    if let Some(detail) =
        maybe_auto_approve_mutations(ctx.config, ctx.store_arc, ctx.github, &queued).await?
    {
        if file_tools::is_mutating_file_tool(tool_name) {
            record_session_file_edit(ctx.session, tool_name, tool_args, &detail);
            store_update_session_runtime(ctx.store, ctx.session).await?;
        }
        emit_progress(
            ctx.progress,
            ChatProgress::ApprovalResolved {
                approval_id: queued.id,
                tool_name: queued.tool_name.clone(),
                approved: true,
                detail: detail.clone(),
            },
        );
        ctx.tool_calls.push(ToolCallSummary {
            tool_name: format!("approval:{}", queued.tool_name),
            output: detail.clone(),
        });
        let body = format_tool_context_message(
            tool_name,
            tool_args,
            true,
            &format!("Auto-approved: {detail}"),
        );
        ctx.llm_messages.push(LlmTurnMessage::tool_result_with_id(
            Some(call.id.clone()),
            tool_name,
            body.clone(),
        ));
        append_message(
            ctx.store,
            ctx.session_id,
            ChatRole::Tool,
            &body,
            Some(tool_name),
            Some(tool_args.to_string()),
        )
        .await?;
        return Ok(MutatingToolOutcome::Continue);
    }
    persist_native_assistant_tool_call(ctx.store, ctx.session_id, ctx.step).await?;
    emit_progress(
        ctx.progress,
        ChatProgress::ApprovalQueued {
            approval_id: queued.id,
            session_id: *ctx.session_id,
            tool_name: queued.tool_name.clone(),
            tool_args_json: tool_args.to_string(),
            description: queued.description.clone(),
        },
    );
    ctx.tool_calls.push(ToolCallSummary {
        tool_name: format!("approval:{}", queued.tool_name),
        output: queued.summary.clone(),
    });
    let pending_body = format!("Mutating tool awaiting approval. {}", queued.summary);
    let body = format_tool_approval_pending_message(tool_name, tool_args, queued.id, &pending_body);
    append_message(
        ctx.store,
        ctx.session_id,
        ChatRole::Tool,
        &body,
        Some(tool_name),
        Some(tool_args.to_string()),
    )
    .await?;
    emit_context_snapshot(
        ctx.progress,
        ctx.llm_messages,
        ctx.llm_rounds,
        ctx.token_budget,
        &ctx.discovery,
        ctx.tool_mode,
        Some((ctx.runtime_panel.0.as_str(), ctx.runtime_panel.1)),
    )
    .await;
    Ok(MutatingToolOutcome::AwaitingApproval)
}

fn record_session_file_edit(
    session: &mut crate::store::ChatSession,
    tool_name: &str,
    tool_args: &Value,
    detail: &str,
) {
    let path = tool_args
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or("?");
    let summary = if detail.contains(path) {
        detail.to_string()
    } else {
        file_tools::format_edit_summary(
            std::path::Path::new(&session.runtime_state.workspace_path),
            path,
            tool_name,
            detail,
        )
    };
    session.runtime_state.recent_edits.push(summary);
    const MAX_EDITS: usize = 8;
    if session.runtime_state.recent_edits.len() > MAX_EDITS {
        let drain = session.runtime_state.recent_edits.len() - MAX_EDITS;
        session.runtime_state.recent_edits.drain(0..drain);
    }
}

async fn store_update_session_runtime(
    store: &dyn Store,
    session: &crate::store::ChatSession,
) -> Result<()> {
    store.update_chat_session(session).await
}

async fn queue_review_fallback_approval(
    store: &dyn Store,
    workspace: &std::path::Path,
    tool_name: &str,
    args: &Value,
    review: &crate::agent::bash_tool::BashCommandReview,
) -> Result<QueuedApproval> {
    use crate::agent::review_gate::approval_kind_for_review_gated_tool;

    let workspace_key = workspace.to_string_lossy().to_string();
    let args_json = serde_json::to_string(args).unwrap_or_else(|_| "{}".to_string());
    let kind = approval_kind_for_review_gated_tool(tool_name).ok_or_else(|| {
        CoworkerError::Workflow(format!("no approval kind for review-gated tool: {tool_name}"))
    })?;
    let description = format_review_rejection_description(tool_name, review);

    let approval = Approval {
        id: Uuid::new_v4(),
        kind,
        repo: workspace_key,
        pr_number: None,
        run_id: None,
        target_branch: None,
        incident_id: None,
        description: description.clone(),
        status: ApprovalStatus::Pending,
        created_at: Utc::now(),
        decided_at: None,
        comment_body: Some(args_json),
        issue_number: None,
        label: None,
    };
    store.push_approval(&approval).await?;
    append_audit(
        store,
        "info",
        "chat",
        &format!(
            "queued LLM-review fallback approval {} ({:?})",
            approval.id, approval.kind
        ),
    )
    .await;
    Ok(QueuedApproval {
        id: approval.id,
        tool_name: tool_name.to_string(),
        description: approval.description.clone(),
        summary: format!(
            "LLM safety review rejected `{tool_name}` — approval {} queued (confirm in Approvals UI).",
            approval.id
        ),
    })
}

#[allow(clippy::too_many_arguments)]
async fn handle_llm_review_rejection(
    store: &dyn Store,
    session_id: &Uuid,
    workspace: &std::path::Path,
    config: &Config,
    store_arc: &Arc<dyn Store>,
    github: &Arc<GithubHarness>,
    progress: &Option<broadcast::Sender<AppEvent>>,
    llm_messages: &mut Vec<LlmTurnMessage>,
    tool_calls: &mut Vec<ToolCallSummary>,
    call_id: &str,
    tool_name: &str,
    tool_args: &Value,
    review: &crate::agent::bash_tool::BashCommandReview,
) -> Result<LlmReviewRejectionOutcome> {
    if !crate::agent::review_gate::is_review_gated_tool(tool_name) {
        return Err(CoworkerError::Workflow(format!(
            "LLM review rejected unknown tool: {tool_name}"
        )));
    }
    let queued =
        queue_review_fallback_approval(store, workspace, tool_name, tool_args, review).await?;
    if let Some(detail) =
        maybe_auto_approve_mutations(config, store_arc, github, &queued).await?
    {
        emit_progress(
            progress,
            ChatProgress::ApprovalResolved {
                approval_id: queued.id,
                tool_name: queued.tool_name.clone(),
                approved: true,
                detail: detail.clone(),
            },
        );
        tool_calls.push(ToolCallSummary {
            tool_name: format!("approval:{}", queued.tool_name),
            output: detail.clone(),
        });
        let body = format_tool_context_message(
            tool_name,
            tool_args,
            true,
            &format!("Auto-approved after LLM reject: {detail}"),
        );
        llm_messages.push(LlmTurnMessage::tool_result_with_id(
            Some(call_id.to_string()),
            tool_name,
            body.clone(),
        ));
        append_message(
            store,
            session_id,
            ChatRole::Tool,
            &body,
            Some(tool_name),
            Some(tool_args.to_string()),
        )
        .await?;
        push_harness_nudge(
            llm_messages,
            format!(
                "Tool `{tool_name}` auto-approved and completed. \
                 Do not repeat the same call — continue or reply to the user."
            ),
        );
        return Ok(LlmReviewRejectionOutcome::AutoApproved { detail });
    }
    emit_progress(
        progress,
        ChatProgress::ApprovalQueued {
            approval_id: queued.id,
            session_id: *session_id,
            tool_name: queued.tool_name.clone(),
            tool_args_json: tool_args.to_string(),
            description: queued.description.clone(),
        },
    );
    tool_calls.push(ToolCallSummary {
        tool_name: format!("approval:{}", queued.tool_name),
        output: queued.summary.clone(),
    });
    let reject_detail = llm_review_reject_detail(tool_name, tool_args, review);
    let pending_body = format!(
        "LLM safety review rejected this action; awaiting human approval. {}\n\n{reject_detail}",
        queued.summary
    );
    let body = format_tool_approval_pending_message(tool_name, tool_args, queued.id, &pending_body);
    append_message(
        store,
        session_id,
        ChatRole::Tool,
        &body,
        Some(tool_name),
        Some(tool_args.to_string()),
    )
    .await?;
    llm_messages.push(LlmTurnMessage::tool_result_with_id(
        Some(call_id.to_string()),
        tool_name,
        body,
    ));
    Ok(LlmReviewRejectionOutcome::AwaitingApproval)
}

fn llm_review_reject_detail(
    tool_name: &str,
    tool_args: &Value,
    review: &crate::agent::bash_tool::BashCommandReview,
) -> String {
    use crate::agent::bash_tool::BASH_RUN_TOOL;
    use crate::agent::file_tools::{EDIT_FILE, WRITE_FILE};
    use crate::agent::harness_errors::{
        bash_safety_reject_envelope, file_edit_safety_reject_envelope,
        python_safety_reject_envelope,
    };
    use crate::agent::python_tool::PYTHON_RUN_TOOL;

    match tool_name {
        BASH_RUN_TOOL => {
            let command = tool_args
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            bash_safety_reject_envelope(command, review).format_harness_nudge()
        }
        PYTHON_RUN_TOOL => {
            let code = tool_args
                .get("code")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            python_safety_reject_envelope(code, review).format_harness_nudge()
        }
        EDIT_FILE | WRITE_FILE => {
            file_edit_safety_reject_envelope(tool_name, tool_args, review).format_harness_nudge()
        }
        _ => format!("verdict={}, reason={}", review.verdict, review.reason_code),
    }
}

enum ReadonlyToolExecuteResult {
    Output(String),
    LlmReviewRejected(crate::agent::bash_tool::BashCommandReview),
}

fn wrap_review_gate(outcome: ReviewGateOutcome) -> ReadonlyToolExecuteResult {
    match outcome {
        ReviewGateOutcome::Executed(s) => ReadonlyToolExecuteResult::Output(s),
        ReviewGateOutcome::LlmRejected(r) => ReadonlyToolExecuteResult::LlmReviewRejected(r),
    }
}

enum LlmReviewRejectionOutcome {
    AutoApproved { detail: String },
    AwaitingApproval,
}

async fn execute_readonly_tool_with_heartbeat(
    store: Arc<dyn Store>,
    github: Arc<GithubHarness>,
    discovery: &Arc<Mutex<ChatDiscoveryState>>,
    progress: &Option<broadcast::Sender<AppEvent>>,
    ctx: ReadonlyToolContext<'_>,
    tool_name: &str,
    tool_args: Value,
) -> Result<ReadonlyToolExecuteResult> {
    let name = tool_name.to_string();
    let args = tool_args.clone();
    let progress = progress.clone();
    let discovery = Arc::clone(discovery);
    let mut tool_fut = Box::pin(execute_readonly_tool(
        store,
        github,
        &discovery,
        ctx,
        tool_name,
        tool_args,
    ));
    let started = Instant::now();
    let mut tick = time::interval(Duration::from_millis(500));
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    tick.tick().await;

    loop {
        tokio::select! {
            result = &mut tool_fut => return result,
            _ = tick.tick() => {
                let detail = format_tool_progress_detail(tool_name, &args, started.elapsed());
                emit_progress(
                    &progress,
                    ChatProgress::ToolProgress {
                        name: name.clone(),
                        detail,
                    },
                );
            }
        }
    }
}

/// Elapsed / paging hint for the TUI while a readonly tool is in flight.
pub(crate) fn format_tool_progress_detail(
    tool_name: &str,
    args: &Value,
    elapsed: Duration,
) -> String {
    let secs = elapsed.as_secs();
    match tool_name {
        "ci_get_failed_logs" => {
            let offset = args
                .get("offset_lines")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let max_lines = args
                .get("max_lines")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            if max_lines > 0 {
                let page = offset
                    .checked_div(max_lines)
                    .unwrap_or(0)
                    .saturating_add(1);
                format!("page {page}, {secs}s")
            } else {
                format!("fetching logs, {secs}s")
            }
        }
        "ci_get_run_summary" | "ci_analyze_pr_failures" | "pr_get_overview" | "pr_get_diff" => {
            format!("{secs}s")
        }
        "bash_run" => {
            let cmd = args
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            format!("{cmd}, {secs}s")
        }
        "python_run" => {
            let lines = args
                .get("code")
                .and_then(|v| v.as_str())
                .unwrap_or("?")
                .lines()
                .next()
                .unwrap_or("?");
            format!("{lines}, {secs}s")
        }
        "web_browser" => {
            let url = args.get("url").and_then(|v| v.as_str()).unwrap_or("?");
            format!("{url}, {secs}s")
        }
        _ => format!("{secs}s"),
    }
}

async fn execute_readonly_tool(
    store: Arc<dyn Store>,
    github: Arc<GithubHarness>,
    discovery: &Arc<Mutex<ChatDiscoveryState>>,
    ctx: ReadonlyToolContext<'_>,
    tool_name: &str,
    mut tool_args: Value,
) -> Result<ReadonlyToolExecuteResult> {
    finalize_tool_args(
        tool_name,
        &mut tool_args,
        ctx.configured_repos,
        ctx.user_task,
    );
    if workflow_harness::is_workflow_harness_tool(tool_name) {
        return Ok(ReadonlyToolExecuteResult::Output(
            workflow_harness::execute_workflow_harness(
                WorkflowHarnessCtx {
                    config: ctx.config.clone(),
                    store,
                    github,
                    llm: ctx.llm.clone(),
                },
                tool_name,
                tool_args,
            )
            .await?,
        ));
    }
    if harness_tools::is_harness_tool(tool_name) {
        return Ok(ReadonlyToolExecuteResult::Output(
            harness_tools::execute_harness_tool(store.as_ref(), tool_name, tool_args).await?,
        ));
    }
    if bash_tool::is_bash_tool(tool_name) {
        return Ok(wrap_review_gate(
            bash_tool::execute_bash_tool(ctx.bash, ctx.llm.as_ref(), ctx.workspace, &tool_args)
                .await?,
        ));
    }
    if python_tool::is_python_tool(tool_name) {
        return Ok(wrap_review_gate(
            python_tool::execute_python_tool(ctx.python, ctx.llm.as_ref(), ctx.workspace, &tool_args)
                .await?,
        ));
    }
    if file_tools::is_mutating_file_tool(tool_name) {
        return Ok(wrap_review_gate(
            file_edit_tool::execute_mutating_file_tool_with_review(
                ctx.workspace,
                ctx.llm.as_ref(),
                tool_name,
                &tool_args,
            )
            .await?,
        ));
    }
    if web_browser_tool::is_web_browser_tool(tool_name) {
        return Ok(ReadonlyToolExecuteResult::Output(
            web_browser_tool::execute_web_browser_tool(
                &ctx.config.chat.web_browser,
                ctx.workspace,
                &tool_args,
            )
            .await?,
        ));
    }
    if file_tools::is_file_tool(tool_name) {
        return Ok(ReadonlyToolExecuteResult::Output(file_tools::execute_file_tool(
            ctx.workspace,
            tool_name,
            &tool_args,
        )?));
    }
    Ok(ReadonlyToolExecuteResult::Output(match tool_name {
        "tool_list" => {
            if let Some(cached) = crate::agent::hooks::tool_list_cached_response(
                &*discovery.lock().await,
            ) {
                return Ok(ReadonlyToolExecuteResult::Output(cached));
            }
            let text = gh_tool(github.as_ref(), "tool_list", json!({})).await?;
            discovery.lock().await.store_tool_list(text.clone());
            text
        }
        "tool_list_category" => {
            let category = tool_args
                .get("category")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    agent_validation_error(
                        "tool_list_category",
                        "TOOL_MISSING_ARG",
                        "tool_list_category needs category",
                        "Pass category from tool_list",
                    )
                })?;
            gh_tool(github.as_ref(), "tool_list_category", json!({ "category": category })).await?
        }
        "tool_search" => {
            let query = tool_args
                .get("query")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    agent_validation_error(
                        "tool_search",
                        "TOOL_MISSING_ARG",
                        "tool_search needs query",
                        "Pass a short tool name keyword",
                    )
                })?;
            let mut args = json!({ "query": query });
            if let Some(limit) = tool_args.get("limit") {
                args["limit"] = limit.clone();
            }
            gh_tool(github.as_ref(), "tool_search", args).await?
        }
        "tool_describe" => {
            let name = tool_args
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    agent_validation_error(
                        "tool_describe",
                        "TOOL_MISSING_ARG",
                        "tool_describe needs name",
                        "Pass exact tool name from tool_search",
                    )
                })?;
            gh_tool(github.as_ref(), "tool_describe", json!({ "name": name })).await?
        }
        "resource_read" => {
            let uri = tool_args
                .get("uri")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    agent_validation_error(
                        "resource_read",
                        "TOOL_MISSING_ARG",
                        "resource_read needs uri",
                        "Use pr:// or ci:// URI from tool_describe",
                    )
                })?;
            read_resource(github.as_ref(), uri).await?
        }
        "skill_load" => {
            let name = tool_args
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    agent_validation_error(
                        "skill_load",
                        "TOOL_MISSING_ARG",
                        "skill_load needs name",
                        "Pass skill name from **Available skills** in the system prompt",
                    )
                })?;
            let mut state = discovery.lock().await;
            let skill = state
                .skill_registry
                .get(name)
                .cloned()
                .ok_or_else(|| {
                    agent_validation_error(
                        "skill_load",
                        "TOOL_NOT_FOUND",
                        format!("unknown skill {name:?}"),
                        "Pick a name from **Available skills** in the system prompt",
                    )
                })?;
            state.warm_skill_tools(&skill);
            SkillRegistry::format_skill_load(&skill)
        }
        "tool_call" => {
            let name = tool_args
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    agent_validation_error(
                        "tool_call",
                        "TOOL_MISSING_ARG",
                        "tool_call needs name",
                        "Pass { \"name\": \"...\", \"args\": { ... } }",
                    )
                })?;
            let args = tool_args.get("args").cloned().unwrap_or_else(|| json!({}));
            if is_mutating_tool(name) {
                return Err(CoworkerError::Workflow(format!(
                    "{name} is mutating — use approval action"
                )));
            }
            gh_tool(github.as_ref(), "tool_call", json!({ "name": name, "args": args })).await?
        }
        other if is_mutating_tool(other) => {
            return Err(CoworkerError::Workflow(format!(
                "{other} is mutating — use approval action"
            )));
        }
        other => gh_tool_with_retry(github.as_ref(), other, tool_args).await?,
    }))
}

async fn queue_mutating_approval(
    store: &dyn Store,
    _workspace: &std::path::Path,
    tool_name: &str,
    args: &Value,
) -> Result<QueuedApproval> {
    let comment_body = args
        .get("body")
        .and_then(|v| v.as_str())
        .map(str::to_string);

    let (kind, description, repo, pr_number, run_id, target_branch, issue_number, label, payload) =
        match tool_name {
        "ci_rerun_workflow" => {
            let run_id = args
                .get("run_id")
                .and_then(|v| v.as_i64())
                .ok_or_else(|| CoworkerError::Workflow("ci_rerun_workflow needs run_id".into()))?;
            let repo = args
                .get("repo")
                .and_then(|v| v.as_str())
                .map(sanitize_repo_string)
                .unwrap_or_else(|| "unknown/repo".to_string());
            (
                ApprovalKind::RerunFlaky,
                format!("Chat: rerun workflow run {run_id} on {repo}"),
                repo,
                None,
                Some(run_id),
                None,
                None,
                None,
                None,
            )
        }
        "pr_create_backport" => {
            let repo = args
                .get("repo")
                .and_then(|v| v.as_str())
                .map(sanitize_repo_string)
                .unwrap_or_else(|| "unknown/repo".to_string());
            let pr_number = args
                .get("pr_number")
                .and_then(|v| v.as_u64())
                .map(|n| n as u32)
                .ok_or_else(|| {
                    CoworkerError::Workflow("pr_create_backport needs pr_number".into())
                })?;
            let target_branch = args
                .get("target_branch")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    CoworkerError::Workflow("pr_create_backport needs target_branch".into())
                })?
                .to_string();
            (
                ApprovalKind::Backport,
                format!("Chat: backport #{pr_number} → {target_branch} on {repo}"),
                repo,
                Some(pr_number),
                None,
                Some(target_branch),
                None,
                None,
                None,
            )
        }
        "pr_post_comment" => {
            let repo = args
                .get("repo")
                .and_then(|v| v.as_str())
                .map(sanitize_repo_string)
                .unwrap_or_else(|| "unknown/repo".to_string());
            let pr_number = args
                .get("pr_number")
                .and_then(|v| v.as_u64())
                .map(|n| n as u32)
                .ok_or_else(|| CoworkerError::Workflow("pr_post_comment needs pr_number".into()))?;
            if comment_body.is_none() {
                return Err(CoworkerError::Workflow("pr_post_comment needs body".into()));
            }
            (
                ApprovalKind::PostComment,
                format!("Chat: post comment on #{pr_number} ({repo})"),
                repo,
                Some(pr_number),
                None,
                None,
                None,
                None,
                None,
            )
        }
        "issue_add_label" => {
            let repo = args
                .get("repo")
                .and_then(|v| v.as_str())
                .map(sanitize_repo_string)
                .unwrap_or_else(|| "unknown/repo".to_string());
            let issue_number = args
                .get("issue_number")
                .and_then(|v| v.as_u64())
                .map(|n| n as u32)
                .ok_or_else(|| {
                    CoworkerError::Workflow("issue_add_label needs issue_number".into())
                })?;
            let label = args
                .get("label")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .ok_or_else(|| CoworkerError::Workflow("issue_add_label needs label".into()))?;
            (
                ApprovalKind::IssueAddLabel,
                format!("Chat: add label `{label}` to issue #{issue_number} ({repo})"),
                repo,
                None,
                None,
                None,
                Some(issue_number),
                Some(label),
                None,
            )
        }
        other => {
            return Err(CoworkerError::Workflow(format!(
                "unknown mutating tool: {other}"
            )));
        }
    };

    let approval = Approval {
        id: Uuid::new_v4(),
        kind,
        repo,
        pr_number,
        run_id,
        target_branch,
        incident_id: None,
        description,
        status: ApprovalStatus::Pending,
        created_at: Utc::now(),
        decided_at: None,
        comment_body: payload.or(comment_body),
        issue_number,
        label,
    };
    store.push_approval(&approval).await?;
    append_audit(
        store,
        "info",
        "chat",
        &format!("queued approval {} ({:?})", approval.id, approval.kind),
    )
    .await;
    Ok(QueuedApproval {
        id: approval.id,
        tool_name: tool_name.to_string(),
        description: approval.description.clone(),
        summary: format!(
            "Approval {} queued for `{tool_name}` — confirm in the approval popup.",
            approval.id
        ),
    })
}

/// When `chat.auto_approve_mutations` is enabled, run the queued mutation immediately.
async fn maybe_auto_approve_mutations(
    config: &Config,
    store: &Arc<dyn Store>,
    github: &Arc<GithubHarness>,
    queued: &QueuedApproval,
) -> Result<Option<String>> {
    if !config.chat.auto_approve_mutations {
        return Ok(None);
    }
    match approvals::process_decision(Arc::clone(store), Arc::clone(github), &queued.id, true).await {
        Ok(detail) => Ok(Some(detail)),
        Err(e) => Err(e),
    }
}

/// Result of queueing a mutating tool for human approval.
#[derive(Debug, Clone)]
pub struct QueuedApproval {
    pub id: Uuid,
    pub tool_name: String,
    pub description: String,
    pub summary: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cap_long_tool_output() {
        use crate::agent::context::cap_tool_result;
        let long = "x".repeat(7000);
        let capped = cap_tool_result("pr_get_overview", &long);
        assert!(capped.len() < 7000);
        assert!(capped.contains("truncated"));
    }

    #[test]
    fn tool_summary_preview_truncates() {
        let long = "x".repeat(200);
        let tc = ToolCallSummary {
            tool_name: "pr_list_open".into(),
            output: long,
        };
        assert_eq!(tc.preview(120).chars().count(), 121); // 120 + ellipsis
        assert!(tc.preview(120).ends_with('…'));
    }

    #[test]
    fn mutating_tools_list() {
        assert!(is_mutating_tool("ci_rerun_workflow"));
        assert!(is_mutating_tool("issue_add_label"));
        assert!(!is_mutating_tool("write_file"));
        assert!(!is_mutating_tool("edit_file"));
        assert!(!is_mutating_tool("pr_list_open"));
        assert!(!is_mutating_tool("read_file"));
    }

    #[test]
    fn push_harness_nudge_replaces_instead_of_stacking() {
        let mut msgs = Vec::new();
        push_harness_nudge(
            &mut msgs,
            "Tool `pr_get_overview` is missing required `repo`.".into(),
        );
        push_harness_nudge(
            &mut msgs,
            "Tool `pr_get_overview` is missing required `repo`.".into(),
        );
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].content.contains("Harness retry 2"));
    }

    #[test]
    fn harness_nudge_stays_in_chronological_order() {
        let mut msgs = vec![
            LlmTurnMessage::new("user", "Rerun failed CIs."),
            LlmTurnMessage::assistant_tool_call(
                String::new(),
                vec![crate::llm::chat::LlmToolCall {
                    id: "call_1".into(),
                    name: "ci_get_failed_logs".into(),
                    arguments: json!({"repo": "acme/widget", "run_id": 1}),
                }],
            ),
            LlmTurnMessage::tool_result("ci_get_failed_logs", "log output"),
        ];
        push_harness_nudge(
            &mut msgs,
            "Identical `ci_get_failed_logs` with the same args was already fetched in this turn."
                .into(),
        );
        msgs.push(LlmTurnMessage::assistant_tool_call(
            String::new(),
            vec![crate::llm::chat::LlmToolCall {
                id: "call_2".into(),
                name: "ci_rerun_workflow".into(),
                arguments: json!({"repo": "acme/widget", "run_id": 1}),
            }],
        ));
        assert_eq!(msgs.len(), 5);
        assert!(matches!(msgs[3].role, "user"));
        assert!(msgs[3].content.contains("Identical `ci_get_failed_logs`"));
        assert!(msgs[4].tool_calls.is_some());
    }

    #[test]
    fn satisfied_missing_arg_nudge_is_removed_after_success() {
        let mut msgs = vec![LlmTurnMessage::new(
            "user",
            "Tool `pr_get_overview` is missing required `repo`.\n\n(Harness retry 2 — call the tool above via the native tool API; no further reasoning.)",
        )];
        remove_satisfied_missing_arg_nudges(
            &mut msgs,
            "pr_get_overview",
            &json!({"repo": "acme/widget", "pr_number": 19263}),
        );
        assert!(msgs.is_empty());
    }

    #[test]
    fn stale_missing_arg_nudge_is_pruned_from_history_context() {
        let mut msgs = vec![
            LlmTurnMessage::new("user", "Tool `pr_get_overview` is missing required `repo`."),
            LlmTurnMessage::tool_result("pr_get_overview", "PR #19263 in acme/widget"),
        ];
        prune_stale_missing_arg_nudges(&mut msgs);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].role, "tool");
        assert!(msgs[0].content.contains("PR #19263"));
    }

    #[test]
    fn tool_failure_nudge_includes_error_and_contract() {
        let catalog = tool_catalog::ToolCatalog::new();
        let args = json!({ "repo": "wrong/repo", "pr_number": 1 });
        let mut msgs = Vec::new();
        maybe_push_tool_failure_harness_nudge(
            &catalog,
            "pr_get_overview",
            &args,
            "failed to get pull request: HTTP 404: Not Found",
            &["acme/widget".into()],
            &mut msgs,
        );
        assert_eq!(msgs.len(), 1);
        let body = &msgs[0].content;
        assert!(body.contains("404"));
        assert!(body.contains("wrong/repo"));
        assert!(body.contains("[Harness]"));
        assert!(body.contains("Try:"));
        assert!(!body.contains("is missing required `repo`"));
    }

    #[test]
    fn tool_failure_nudge_uses_schema_when_args_incomplete() {
        let catalog = tool_catalog::ToolCatalog::new();
        let args = json!({ "repo": "acme/widget" });
        let msg = catalog.format_tool_failure_nudge(
            "pr_get_overview",
            &args,
            "failed to get pull request",
            &["acme/widget".into()],
        );
        assert!(msg.contains("Call `pr_get_overview`"));
    }

    #[test]
    fn duplicate_tool_nudge_is_generic() {
        let nudge = duplicate_tool_nudge(
            "pr_list_changed_files",
            DuplicateToolBlock::AlreadySucceeded,
        );
        assert!(nudge.contains("already fetched"));
        assert!(!nudge.contains("19258"));
    }

    #[test]
    fn tool_transcript_matches_prior_args() {
        let args = json!({"name": "pr-review"});
        let content = format_tool_context_message(
            "skill_load",
            &args,
            true,
            "### pr-review\nbody",
        );
        assert!(tool_transcript_matches_args(
            &content,
            &args,
            &canonical_tool_args(&args),
        ));
    }

    #[test]
    fn find_prior_tool_result_body_from_messages() {
        let args = json!({"repo": "acme/widget", "pr_number": 42});
        let body = format_tool_context_message("pr_get_overview", &args, true, "PR ok");
        let msgs = vec![LlmTurnMessage::tool_result_with_id(
            Some("call_1".into()),
            "pr_get_overview",
            body,
        )];
        let found = find_prior_tool_result_body(&msgs, "pr_get_overview", &args);
        assert!(found.is_some());
        assert!(found.unwrap().contains("PR ok"));
    }

    #[test]
    fn sanitize_repo_strips_display_prefix() {
        assert_eq!(sanitize_repo_string("repo=acme/widget"), "acme/widget");
        assert_eq!(sanitize_repo_string("  acme/widget  "), "acme/widget");
        assert_eq!(
            sanitize_repo_string("https://github.com/acme/widget/pull/1"),
            "acme/widget"
        );
        let mut args = json!({ "repo": "repo=acme/widget", "run_id": 27590223890_i64 });
        normalize_pr_tool_args("ci_rerun_workflow", &mut args);
        assert_eq!(args["repo"], json!("acme/widget"));
    }

    #[test]
    fn coerce_string_pr_number() {
        let mut args = json!({ "pr_number": "19252", "repo": "o/r" });
        coerce_numeric_tool_args("pr_get_overview", &mut args);
        assert_eq!(args["pr_number"], json!(19252));
    }

    #[test]
    fn tool_call_normalization_preserves_nested_args() {
        let mut args = json!({
            "name": "pr_get_overview",
            "args": { "repo": "o/r", "pr_number": 19263 }
        });
        normalize_model_tool_args("tool_call", &mut args);
        assert_eq!(args["name"], json!("pr_get_overview"));
        assert_eq!(args["args"]["repo"], json!("o/r"));
        assert_eq!(args["args"]["pr_number"], json!(19263));
    }

    #[test]
    fn tool_call_normalization_moves_loose_target_args_under_args() {
        let mut args = json!({
            "name": "pr_get_overview",
            "repo": "o/r",
            "pr_number": 19263
        });
        normalize_model_tool_args("tool_call", &mut args);
        assert!(args.get("repo").is_none());
        assert!(args.get("pr_number").is_none());
        assert_eq!(args["args"]["repo"], json!("o/r"));
        assert_eq!(args["args"]["pr_number"], json!(19263));
    }

    #[test]
    fn write_file_normalizes_new_string_to_content() {
        let mut args = json!({
            "path": "tmp/app.py",
            "new_string": "print('hi')\n"
        });
        normalize_model_tool_args("write_file", &mut args);
        assert_eq!(args["content"], json!("print('hi')\n"));
    }

    #[test]
    fn finalize_tool_call_autofills_repo_and_pr() {
        let repos = vec!["acme/widget".to_string()];
        let mut args = json!({
            "name": "pr_get_overview",
            "args": {}
        });
        finalize_tool_args(
            "tool_call",
            &mut args,
            &repos,
            "Why is CI failing on PR #19264?",
        );
        assert_eq!(args["args"]["repo"], json!("acme/widget"));
        assert_eq!(args["args"]["pr_number"], json!(19264));
    }

    #[test]
    fn tool_call_json_object_nudge() {
        let catalog = tool_catalog::ToolCatalog::new();
        let args = json!({ "name": "pr_get_overview", "args": "not-an-object" });
        let mut msgs = Vec::new();
        maybe_push_tool_failure_harness_nudge(
            &catalog,
            "tool_call",
            &args,
            "args must be a JSON object",
            &["acme/widget".into()],
            &mut msgs,
        );
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].content.contains("JSON object"));
        assert!(msgs[0].content.contains("pr_get_overview"));
    }

    #[test]
    fn format_tool_args_short_includes_fields() {
        let args = json!({
            "repo": "acme/widget",
            "pr": 42,
            "extra": "x"
        });
        let short = format_tool_args_short(&args);
        assert!(short.contains("repo=acme/widget"));
        assert!(short.contains("pr=42"));
        assert!(short.contains("extra=x"));
    }

    #[test]
    fn chat_limit_zero_means_unlimited() {
        assert!(!chat_limit_reached(0, 100));
        assert!(chat_limit_reached(8, 8));
        assert!(!chat_limit_reached(8, 7));
    }

    #[test]
    fn turn_exhausted_reply_includes_tool_summaries() {
        let msg = synthesize_turn_exhausted_reply(
            &[ToolCallSummary {
                tool_name: "ci_analyze_pr_failures".into(),
                output: "2 failing runs".into(),
            }],
            "why CI fails",
            ChatStopReason::Duration { secs: 900 },
        );
        assert!(msg.contains("900s"));
        assert!(msg.contains("ci_analyze_pr_failures"));
    }

    #[test]
    fn build_context_snapshot_includes_tool_call_body() {
        use crate::llm::chat::LlmToolCall;
        let budget = TokenBudget::from_config(64_000);
        let msgs = vec![
            LlmTurnMessage::new("user", "check PR"),
            LlmTurnMessage::assistant_tool_call(
                "",
                vec![LlmToolCall {
                    id: "call_1".into(),
                    name: "ci_get_failed_logs".into(),
                    arguments: json!({"repo": "acme/widget", "run_id": 99}),
                }],
            ),
        ];
        let snap = build_context_snapshot(&msgs, 1, &budget, &[], &[], None);
        let assistant = snap.messages.last().expect("assistant line");
        assert_eq!(assistant.display_role, "assistant");
        assert!(assistant.content.contains("tool_call: ci_get_failed_logs"));
        assert!(assistant.tokens > 0);
    }

    #[test]
    fn build_context_snapshot_includes_final_assistant_reply() {
        use crate::llm::LlmTurnMessage;
        let budget = TokenBudget::from_config(64_000);
        let msgs = vec![
            LlmTurnMessage::new("user", "What failed in CI?"),
            LlmTurnMessage::new("assistant", "Run 123 failed due to a DB auth error."),
        ];
        let snap = build_context_snapshot(&msgs, 3, &budget, &[], &[], None);
        let last = snap.messages.last().expect("assistant reply");
        assert_eq!(last.display_role, "assistant");
        assert!(last.content.contains("DB auth error"));
    }

    #[test]
    fn build_context_snapshot_counts_messages() {
        use crate::llm::LlmTurnMessage;
        let budget = TokenBudget::from_config(64_000);
        let msgs = vec![
            LlmTurnMessage::new("system", "You are helpful."),
            LlmTurnMessage::tool_result("pr_list_open", "#1 title"),
        ];
        let snap = build_context_snapshot(&msgs, 2, &budget, &[], &[], None);
        assert_eq!(snap.message_count, 2);
        assert_eq!(snap.messages.len(), 2);
        assert_eq!(snap.turn, 2);
        assert_eq!(snap.context_limit, 64_000);
        assert_eq!(snap.input_budget, 52_523);
        assert!(snap.message_tokens > 0);
        assert_eq!(snap.messages[0].display_role, "system");
        assert_eq!(snap.messages[1].display_role, "tool");
        assert!(snap.messages[1].content.contains("#1 title"));
        assert!(snap.messages[1].tokens > 0);
    }

    #[test]
    fn build_context_snapshot_includes_tools_tokens() {
        let budget = TokenBudget::from_config(64_000);
        let tools = vec![json!({
            "type": "function",
            "function": {
                "name": "pr_get_overview",
                "description": "PR snapshot",
                "parameters": { "type": "object" }
            }
        })];
        let snap = build_context_snapshot(
            &[LlmTurnMessage::new("user", "hi")],
            1,
            &budget,
            &tools,
            &[],
            None,
        );
        assert!(snap.tools_tokens > 0);
        assert!(snap.tools_body.contains("pr_get_overview"));
        assert_eq!(snap.tool_names, vec!["pr_get_overview".to_string()]);
        assert!(snap.total_tokens() > snap.message_tokens);
    }

    #[test]
    fn build_context_snapshot_lists_loaded_skills() {
        use crate::engine::SkillSpec;
        let budget = TokenBudget::from_config(64_000);
        let skills = vec![SkillSpec {
            name: "ci-triage".into(),
            description: String::new(),
            body: "## Rules\n- classify CI".into(),
            skill_refs: vec![],
            tool_refs: vec![],
            always_load: false,
            ..Default::default()
        }];
        let system = "Agent body\n\n## Techniques\nhidden\n\n## Context\nrepos: x";
        let snap = build_context_snapshot(
            &[LlmTurnMessage::new("system", system)],
            1,
            &budget,
            &[],
            &skills,
            None,
        );
        assert_eq!(snap.skill_blocks.len(), 1);
        assert!(snap.skill_blocks[0].body.contains("classify CI"));
        assert!(snap.messages[0].content.contains("Agent body"));
        assert!(!snap.messages[0].content.contains("Techniques"));
        assert!(snap.skills_tokens > 0);
    }

    #[test]
    fn build_context_snapshot_preserves_long_system_body() {
        use crate::llm::LlmTurnMessage;
        let budget = TokenBudget::from_config(64_000);
        let body = "x".repeat(20_000);
        let msgs = vec![LlmTurnMessage::new("system", body.clone())];
        let snap = build_context_snapshot(&msgs, 1, &budget, &[], &[], None);
        assert_eq!(snap.messages[0].content.len(), body.len());
        assert_eq!(snap.messages[0].content, body);
    }

    #[test]
    fn build_context_snapshot_panel_shows_full_runtime_not_llm_delta() {
        use crate::engine::format_session_context_message;
        use crate::llm::LlmTurnMessage;
        let budget = TokenBudget::from_config(64_000);
        let delta = "(runtime context revision 2)\n## Store updates\n+ Pending approvals: 2";
        let full = "## Configured repos\no/r\n\n## Local store snapshot\nPending approvals: 2";
        let msgs = vec![LlmTurnMessage::new(
            "user",
            format_session_context_message(delta),
        )];
        let snap = build_context_snapshot(&msgs, 1, &budget, &[], &[], Some((full, 2)));
        assert_eq!(snap.runtime_context_revision, Some(2));
        assert!(snap.messages[0].content.contains("Local store snapshot"));
        assert!(!snap.messages[0].content.contains("Store updates"));
    }

    #[test]
    fn context_display_role_labels_tool_and_harness() {
        assert_eq!(context_display_role("tool", "#1 title"), "tool");
        assert_eq!(
            context_display_role("user", "tool_result(pr_list_open):\n#1"),
            "tool"
        );
        assert_eq!(
            context_display_role("user", "Identical `pr_list_open` with the same args"),
            "harness"
        );
        assert_eq!(context_display_role("system", "You are helpful"), "system");
        assert_eq!(
            context_display_role("user", "[session context]\nrepos: x"),
            "context"
        );
        assert_eq!(context_display_role("user", "list open PRs"), "user");
    }

    #[test]
    fn format_tool_progress_detail_for_paged_logs() {
        let args = json!({"offset_lines": 160, "max_lines": 80});
        let detail = format_tool_progress_detail("ci_get_failed_logs", &args, Duration::from_secs(12));
        assert_eq!(detail, "page 3, 12s");

        let args = json!({});
        let detail = format_tool_progress_detail("ci_get_failed_logs", &args, Duration::from_secs(4));
        assert_eq!(detail, "fetching logs, 4s");
    }

    #[test]
    fn flow_activity_tools_are_transient() {
        assert!(is_flow_activity_tool("skill_load"));
        assert!(is_flow_activity_tool("tool_call"));
        assert!(!is_flow_activity_tool("pr_get_overview"));
    }

    #[test]
    fn format_skill_bootstrap_lists_warm_tools() {
        use crate::engine::SkillSpec;
        let text = format_skill_bootstrap_flow(&[SkillSpec {
            name: "ci-triage".into(),
            description: String::new(),
            body: String::new(),
            skill_refs: vec![],
            tool_refs: vec!["pr_get_ci_snapshot".into()],
            always_load: false,
            ..Default::default()
        }]);
        assert!(text.contains("ci-triage"));
        assert!(text.contains("pr_get_ci_snapshot"));
    }

    #[test]
    fn format_flow_tool_call_shows_inner_name() {
        let args = json!({
            "name": "pr_get_overview",
            "args": {"repo": "o/r", "pr_number": 1}
        });
        let text = format_flow_tool_start("tool_call", &args);
        assert!(text.contains("tool_call → pr_get_overview"));
    }

    #[test]
    fn chat_progress_display_lines() {
        let start = ChatProgress::ToolStart {
            name: "pr_get_overview".into(),
            args_short: "repo=acme/widget, pr=1".into(),
        };
        assert_eq!(
            start.display_line(),
            "  → pr_get_overview(repo=acme/widget, pr=1)"
        );
        let done = ChatProgress::ToolDone {
            name: "pr_get_overview".into(),
            args_short: "repo=acme/widget, pr=1".into(),
            ok: true,
            elapsed_ms: 820,
            output_preview: String::new(),
        };
        assert_eq!(
            done.display_line(),
            "  ✓ pr_get_overview(repo=acme/widget, pr=1) (820ms)"
        );
        let dup = ChatProgress::DuplicateToolBlocked {
            tool_name: "pr_get_overview".into(),
            args_short: "repo=acme/widget, pr=19235".into(),
            attempt: 2,
        };
        assert_eq!(
            dup.display_line(),
            "  ⚠ duplicate pr_get_overview(repo=acme/widget, pr=19235) (attempt 2)"
        );
        assert_eq!(
            dup.status_text(),
            "chat: duplicate pr_get_overview (attempt 2)"
        );
        let reasoning = ChatProgress::ReasoningSummary {
            preview: "Checked CI on PR #42".into(),
            body: "Checked CI on PR #42".into(),
        };
        assert_eq!(
            reasoning.display_line(),
            "  … reasoning: Checked CI on PR #42"
        );
    }

    #[test]
    fn duplicate_tool_block_after_success_or_two_failures() {
        let ok = ToolExecRecord {
            succeeded: true,
            fail_count: 0,
        };
        assert_eq!(
            duplicate_tool_block_reason(Some(&ok)),
            Some(DuplicateToolBlock::AlreadySucceeded)
        );

        let one_fail = ToolExecRecord {
            succeeded: false,
            fail_count: 1,
        };
        assert_eq!(duplicate_tool_block_reason(Some(&one_fail)), None);

        let two_fail = ToolExecRecord {
            succeeded: false,
            fail_count: 2,
        };
        assert_eq!(
            duplicate_tool_block_reason(Some(&two_fail)),
            Some(DuplicateToolBlock::FailedTooMany)
        );
    }

    #[test]
    fn failed_tool_output_allows_identical_retry() {
        let fp = tool_call_fingerprint("pr_list_open", &json!({"repo": "acme/widget"}));
        let err_text = "failed to list pull requests: GitHub returned a temporary server error (HTTP 504: 504 Gateway Timeout)";
        assert!(tool_output_indicates_failure("pr_list_open", err_text));
        let mut records = HashMap::new();
        records.insert(
            fp.clone(),
            ToolExecRecord {
                succeeded: false,
                fail_count: 1,
            },
        );
        assert_eq!(duplicate_tool_block_reason(records.get(&fp)), None);
    }

    #[test]
    fn mcp_error_body_is_not_success_output() {
        let ok = "open PR(s) in acme/widget (2):\n#1 title @x CI:passing review:none";
        assert!(!tool_output_indicates_failure("pr_list_open", ok));
    }

    #[test]
    fn pr_diff_body_may_contain_error_like_substrings() {
        let diff = "Diff for acme/widget#19278 (120 bytes):\n\n\
+++ b/spec/oauth2_spec.lua\n\
+  if status == 429 then -- rate limit exceeded\n\
+  return nil, 'HTTP 500 internal error'";
        assert!(!tool_output_indicates_failure("pr_get_diff", diff));
    }

    #[test]
    fn format_tool_args_short_truncates() {
        let args = json!({
            "repo": "acme/widget",
            "pr_number": 42,
            "extra": "x"
        });
        let short = format_tool_args_short(&args);
        assert!(short.contains("repo=acme/widget"));
        assert!(short.contains("pr_number=42"));
        assert!(short.contains("extra=x"));
    }

    #[test]
    fn tool_fingerprint_is_canonical() {
        let a = json!({"pr_number": 19264, "repo": "acme/widget"});
        let b = json!({"repo": "acme/widget", "pr_number": 19264});
        assert_eq!(
            tool_call_fingerprint("pr_list_changed_files", &a),
            tool_call_fingerprint("pr_list_changed_files", &b)
        );
    }

    #[test]
    fn tool_fingerprint_ignores_extra_keys_and_pr_alias() {
        let a = json!({"repo": "acme/widget", "pr_number": 19264});
        let b = json!({
            "repo": "acme/widget",
            "pr": 19264,
            "extra": "ignored"
        });
        let mut normalized = b.clone();
        normalize_pr_tool_args("pr_list_changed_files", &mut normalized);
        assert_eq!(
            tool_call_fingerprint("pr_list_changed_files", &a),
            tool_call_fingerprint("pr_list_changed_files", &normalized)
        );
    }

    #[test]
    fn python_run_fingerprint_normalizes_whitespace() {
        let a = json!({"code": "import os\n\nprint(1)"});
        let b = json!({"code": "import os\nprint(1)\n"});
        assert_eq!(
            tool_call_fingerprint("python_run", &a),
            tool_call_fingerprint("python_run", &b)
        );
    }

    #[test]
    fn rehydrate_tool_exec_records_from_successful_python_run() {
        let args = json!({"code": "print('ok')"});
        let msg = ChatMessage {
            id: Uuid::new_v4(),
            session_id: Uuid::new_v4(),
            role: ChatRole::Tool,
            content: "tool_result(python_run):\nreview: APPROVE (HUMAN_APPROVE)\nexit: 0 (56ms)\nstdout:\nok"
                .into(),
            ts: Utc::now(),
            tool_name: Some("python_run".into()),
            tool_calls_json: Some(args.to_string()),
        };
        let records = rehydrate_tool_exec_records_from_messages(&[msg]);
        let fp = tool_call_fingerprint("python_run", &args);
        assert_eq!(
            duplicate_tool_block_reason(records.get(&fp)),
            Some(DuplicateToolBlock::AlreadySucceeded)
        );
    }
}
