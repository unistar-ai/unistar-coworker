use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Utc;
use serde_json::{json, Value};
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::agent::bash_tool;
use crate::agent::budget::TokenBudget;
use crate::agent::chat_discovery::ChatDiscoveryState;
use crate::agent::chat_duplicate::{
    duplicate_tool_block_reason, fulfill_duplicate_readonly_tool, harness_retry_or_stop,
    maybe_block_duplicate_tool_call, prune_stale_missing_arg_nudges, push_harness_nudge,
    DuplicateToolBlock, MAX_HARNESS_CORRECTIONS,
};
use crate::agent::chat_stream::persist_interim_assistant_message;

use crate::agent::context::{
    estimate_message_tokens, estimate_tools_tokens, format_system_for_context_panel,
    format_tool_approval_pending_message, format_tool_context_message,
    format_tools_for_context_panel, history_token_budget, message_budget_for_tools,
    pack_session_history_with_llm, skill_body_for_context_panel, tool_names_from_definitions,
    trim_llm_messages_with_llm, trim_system_content, truncate_chars,
};
use crate::agent::file_tools;
use crate::agent::hooks::{HookRunner, TurnContext};
use crate::agent::parse::parse_failing_runs;
use crate::agent::python_tool;
use crate::agent::runtime_context::{
    build_message_focus_lines, build_workspace_git_summary, load_workspace_agents_md,
    plan_runtime_context, RuntimeContextInput,
};
use crate::agent::tool_catalog;
use crate::app::AppEvent;
use crate::config::{ChatToolMode, Config};
use crate::engine::SkillSpec;
use crate::engine::{
    compose_chat_system_prompt, format_session_context_message, load_chat_prompt_bundle_for_session,
};
use crate::error::{CoworkerError, Result};
use crate::github::{effective_chat_tool_mode, GithubHarness};
use crate::llm::chat::{
    reply_premature_for_task, reply_premature_nudge, ChatAgentStep, LlmToolCall, ResolvedToolCall,
};
use crate::llm::{ChatAgentAction, ChatStepOptions, LlmClient, LlmTurnMessage};
use crate::mcp::McpPool;
use crate::store::{ChatMessage, ChatRole, Store};
use tokio::sync::Mutex;

use crate::agent::chat_mutating::{
    handle_mutating_tool_call, maybe_auto_approve_mutations, queue_review_fallback_approval,
    MutatingToolContext, MutatingToolOutcome,
};
use crate::agent::chat_readonly::{
    execute_readonly_tools_parallel, record_tool_outcome, ReadonlyToolContext, ReadonlyToolHarness,
    ReadonlyToolOutcome,
};

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
    /// Raw (uncompressed) thinking trace for reasoning rows, when compression
    /// was applied. The LLM only receives `content` (summary).
    pub reasoning_original: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ContextSkillBlock {
    pub name: String,
    pub body: String,
    pub tokens: u32,
    /// Frontmatter `description` — shown in the context panel skill preview.
    pub description: String,
    /// Frontmatter `always: true` — always-on skills are flagged in the preview.
    pub always: bool,
    /// Frontmatter `skills:` refs (technique skills this skill pulls in).
    pub skills: Vec<String>,
    /// Frontmatter `tools:` refs (business/harness tools this skill declares).
    pub tools: Vec<String>,
    /// Frontmatter `argument-hint` — usage cue shown in the preview.
    pub argument_hint: String,
    /// Frontmatter `intent_phrases` — lazy-routing trigger phrases.
    pub intent_phrases: Vec<String>,
    /// Frontmatter `intent_bonus_keywords` — bonus scoring substrings.
    pub intent_bonus_keywords: Vec<String>,
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
    /// Count from `[N earlier message(s) omitted …]` markers in the LLM list.
    pub context_trimmed_turns: u32,
    /// Human-readable trim/summary note for the context panel.
    pub context_summary_note: Option<String>,
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
        /// Raw (uncompressed) thinking trace, when it differs from `body`.
        /// `None` when no LLM compression occurred (body == original).
        original: Option<String>,
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
        "tool_list"
            | "tool_list_category"
            | "tool_search"
            | "tool_describe"
            | "tool_call"
            | "resource_read"
    )
}

pub(crate) fn activity_flow_kind_for_tool(name: &str) -> ActivityFlowKind {
    if matches!(name, "skill_load") {
        ActivityFlowKind::Skill
    } else {
        ActivityFlowKind::Github
    }
}

pub(crate) fn emit_activity_flow(
    progress: &Option<broadcast::Sender<AppEvent>>,
    kind: ActivityFlowKind,
    text: impl Into<String>,
) {
    let body = text.into();
    if body.trim().is_empty() {
        return;
    }
    emit_progress(progress, ChatProgress::ActivityFlow { kind, text: body });
}

pub(crate) fn emit_activity_flow_clear(progress: &Option<broadcast::Sender<AppEvent>>) {
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

pub(crate) fn format_flow_tool_start(name: &str, args: &Value) -> String {
    let args_short = format_tool_args_short(args);
    let header = match name {
        "tool_call" => {
            let inner = args.get("name").and_then(|v| v.as_str()).unwrap_or("?");
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

pub(crate) fn format_flow_tool_done(name: &str, args: &Value, ok: bool, preview: &str) -> String {
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

pub(crate) fn emit_progress(progress: &Option<broadcast::Sender<AppEvent>>, event: ChatProgress) {
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
    reasoning_originals: &HashMap<String, String>,
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
                description: s.description.clone(),
                always: s.always_load,
                skills: s.skill_refs.clone(),
                tools: s.tool_refs.clone(),
                argument_hint: s.argument_hint.clone(),
                intent_phrases: s.intent_phrases.clone(),
                intent_bonus_keywords: s.intent_bonus_keywords.clone(),
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
                && raw
                    .trim_start()
                    .starts_with(crate::engine::SESSION_CONTEXT_PREFIX)
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
                reasoning_original: if crate::agent::context::is_reasoning_summary_content(&m.content)
                {
                    reasoning_originals.get(&m.content).cloned()
                } else {
                    None
                },
            }
        })
        .collect();
    let (context_trimmed_turns, context_summary_note) =
        crate::agent::context::analyze_context_trim_metadata(messages);
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
        context_trimmed_turns,
        context_summary_note,
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

pub(crate) async fn emit_context_snapshot(
    progress: &Option<broadcast::Sender<AppEvent>>,
    messages: &[LlmTurnMessage],
    turn: u32,
    token_budget: &TokenBudget,
    discovery: &Arc<Mutex<ChatDiscoveryState>>,
    tool_mode: ChatToolMode,
    runtime_panel: Option<(&str, u64)>,
    reasoning_originals: &HashMap<String, String>,
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
            reasoning_originals,
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

pub(crate) fn ensure_chat_not_cancelled(cancel: &Option<Arc<AtomicBool>>) -> Result<()> {
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

pub(crate) async fn race_chat_cancel<T, F>(cancel: Option<Arc<AtomicBool>>, fut: F) -> Result<T>
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
    mcp: Arc<McpPool>,
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
        mcp,
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
    mcp: Arc<McpPool>,
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
        &config.chat.prompt,
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
    let focus_lines = build_message_focus_lines(store.as_ref(), user_task, &config.repos).await?;
    let prev_state =
        if session.runtime_state.revision > 0 || !session.runtime_state.workspace_path.is_empty() {
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
    let mut discovery_state =
        ChatDiscoveryState::with_bootstrap(user_task, skill_registry, &prompt_bundle.skills);
    discovery_state.rehydrate_from_tool_history(&history);
    for tool in crate::engine::SkillRegistry::collect_tool_refs(&prompt_bundle.skills) {
        discovery_state
            .warm_tool_from_registry(&tool, mcp.as_ref())
            .await;
    }
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

    // Inject the Available skills catalog as a separate system message so it
    // isn't subject to trim_system_content's budget cuts. The catalog lists
    // every skill's name + description; the model uses it to pick skill_load
    // targets. Keeping it separate means a large skill set won't cause
    // mid-word truncation of the core system prompt.
    let catalog = prompt_bundle.skill_catalog.trim();
    if !catalog.is_empty() {
        llm_messages.push(LlmTurnMessage::new(
            "system",
            format!("## Available skills\n\n{catalog}"),
        ));
    }
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
    // Duplicate guard is scoped to this user-message turn only. Do not rehydrate from
    // session history — PR/CI snapshots go stale after pushes and users re-fetch.
    let mut tool_exec_records: HashMap<String, ToolExecRecord> = HashMap::new();
    let mut duplicate_tool_nudges: HashMap<String, u32> = HashMap::new();
    let mut duplicate_ui_shown: HashSet<String> = HashSet::new();
    let mut duplicate_forced_reply_nudged = false;
    let mut harness_corrections = 0u32;
    let turn_started = Instant::now();
    let mut llm_rounds = 0u32;
    let config_arc = Arc::new(config.clone());
    let hook_runner = HookRunner::builtin();
    let mut reasoning_originals = crate::agent::context::reasoning_originals_from_history(&history);

    emit_context_snapshot(
        &progress,
        &llm_messages,
        0,
        &token_budget,
        &discovery,
        tool_mode,
        Some((runtime_panel.0.as_str(), runtime_panel.1)),
        &reasoning_originals,
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
            &reasoning_originals,
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
            reasoning_only_warn_secs: config.chat.reasoning_only_warn_secs,
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
            let will_compress = crate::llm::chat::should_compress_reasoning(
                config.chat.compress_reasoning,
                raw,
                config.chat.reasoning_compress_min_chars,
            );
            if will_compress {
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
                // Store the raw (uncompressed) thinking whenever compression was
                // attempted — even if the summarizer failed and we fell back to
                // the original verbatim (summary == raw), the user may still
                // want to view the raw thinking trace.
                let original = if will_compress { Some(raw.as_str()) } else { None };
                persist_reasoning_summary(
                    store.as_ref(),
                    &session.id,
                    &progress,
                    &content,
                    original,
                )
                .await?;
                if let Some(orig) = original {
                    reasoning_originals.insert(content, orig.to_string());
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
                &reasoning_originals,
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
                    &reasoning_originals,
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
                    if tool_allows_repeat_fetch(&call.name) {
                        return None;
                    }
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
                        && fulfill_duplicate_readonly_tool(&mut round, &step, &call, &prepared)
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

                let (mut readonly, mut mutating): (Vec<_>, Vec<_>) = (Vec::new(), Vec::new());
                for call in prepared {
                    if is_mutating_tool(&call.name) || mcp.is_mcp_mutating(&call.name).await {
                        mutating.push(call);
                    } else {
                        readonly.push(call);
                    }
                }

                if !readonly.is_empty() {
                    tools_used += readonly.len() as u32;
                    let outcomes = execute_readonly_tools_parallel(
                        ReadonlyToolHarness {
                            store: store.clone(),
                            github: github.clone(),
                            mcp: mcp.clone(),
                            discovery: discovery.clone(),
                            cancel: cancel.clone(),
                            progress: progress.clone(),
                        },
                        ReadonlyToolContext {
                            configured_repos: &config.repos,
                            user_task,
                            bash: &config.chat.bash,
                            python: &config.chat.python,
                            workspace: &workspace,
                            llm: Arc::clone(&llm),
                            config: Arc::clone(&config_arc),
                            progress: progress.clone(),
                            cancel: cancel.clone(),
                        },
                        readonly,
                    )
                    .await?;
                    let mut turn_awaiting_approval = false;
                    for outcome in outcomes {
                        if let Some(review) = outcome.llm_review_rejected.clone() {
                            let PreparedToolCall { id, name, args, .. } = &outcome.call;
                            match handle_llm_review_rejection(
                                store.as_ref(),
                                &session_id,
                                &workspace,
                                config,
                                &store,
                                &github,
                                &mcp,
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
                                        record_session_file_edit(&mut session, name, args, &detail);
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
                                        duplicate_forced_reply_nudged:
                                            &mut duplicate_forced_reply_nudged,
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
                                        Some(&mcp),
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
                        record_tool_outcome(&mut round, outcome, Some(&mcp)).await?;
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
                            state.warm_tool_from_registry(&name, mcp.as_ref()).await;
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
                        &reasoning_originals,
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
                            mcp: &mcp,
                            progress: &progress,
                            llm_messages: &mut llm_messages,
                            tool_calls: &mut tool_calls,
                            llm_rounds,
                            token_budget: &token_budget,
                            discovery: discovery.clone(),
                            tool_mode,
                            runtime_panel: runtime_panel.clone(),
                            reasoning_originals: &reasoning_originals,
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
        &reasoning_originals,
    )
    .await;
    Ok(ChatTurnResult {
        session_id: session.id,
        assistant_message: fallback,
        tool_calls,
        awaiting_approval: false,
    })
}

pub(crate) async fn append_message(
    store: &dyn Store,
    session_id: &Uuid,
    role: ChatRole,
    content: &str,
    tool_name: Option<&str>,
    tool_calls_json: Option<String>,
    reasoning_original: Option<&str>,
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
            reasoning_original: reasoning_original.map(str::to_string),
        })
        .await
}

pub(crate) async fn persist_native_assistant_tool_call(
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
        None,
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

pub(crate) fn is_mutating_tool(name: &str) -> bool {
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
pub(crate) fn finalize_tool_args(
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
        let has_path = tool_args
            .get("path")
            .and_then(|v| v.as_str())
            .is_some_and(|s| !s.trim().is_empty());
        tool_args["max_bytes"] = json!(if has_path { 64_000 } else { 48_000 });
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
        "ci_get_run_summary"
            | "ci_get_failed_logs"
            | "ci_rerun_workflow"
            | "ci_failure_fingerprint"
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
pub(crate) struct ToolExecRecord {
    pub(crate) succeeded: bool,
    pub(crate) fail_count: u32,
}

/// MCP may return tool errors as plain text; treat them as failures for dedup / retry.
///
/// Only inspects the header / first line. Large payloads such as `pr_get_diff` often
/// contain error-like substrings inside added lines in the unified diff.
pub(crate) fn tool_output_indicates_failure(tool_name: &str, output: &str) -> bool {
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

/// Live GitHub/CI reads can go stale; always execute fresh instead of replaying cache.
fn tool_allows_repeat_fetch(tool_name: &str) -> bool {
    !is_mutating_tool(tool_name) && tool_requires_pr_number(tool_name)
}

fn tool_call_names(tool_calls: &[ToolCallSummary]) -> Vec<&str> {
    tool_calls.iter().map(|tc| tc.tool_name.as_str()).collect()
}

pub(crate) fn tool_call_fingerprint(tool_name: &str, tool_args: &Value) -> String {
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
    if tool_name == "pr_get_diff" {
        if let Some(path) = tool_args
            .get("path")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return Some(format!(
                "{tool_name}:repo={repo},pr_number={pr},path={path}"
            ));
        }
    }
    Some(format!("{tool_name}:repo={repo},pr_number={pr}"))
}

pub(crate) fn canonical_tool_args(value: &Value) -> String {
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

#[cfg(test)]
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

#[cfg(test)]
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

pub(crate) fn ci_analyze_lacks_runs(output: &str) -> bool {
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

async fn persist_reasoning_summary(
    store: &dyn Store,
    session_id: &Uuid,
    progress: &Option<broadcast::Sender<AppEvent>>,
    content: &str,
    original: Option<&str>,
) -> Result<()> {
    append_message(
        store,
        session_id,
        ChatRole::Reasoning,
        content,
        None,
        None,
        original,
    )
    .await?;
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
            original: original.map(str::to_string),
        },
    );
    Ok(())
}

#[derive(Debug, Clone)]
pub(crate) struct PreparedToolCall {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) args: Value,
    pub(crate) fingerprint: String,
}

pub(crate) struct ToolRoundState<'a> {
    pub(crate) harness_corrections: &'a mut u32,
    pub(crate) progress: &'a Option<broadcast::Sender<AppEvent>>,
    pub(crate) store: &'a dyn Store,
    pub(crate) session_id: &'a Uuid,
    pub(crate) llm_messages: &'a mut Vec<LlmTurnMessage>,
    pub(crate) duplicate_tool_nudges: &'a mut HashMap<String, u32>,
    pub(crate) duplicate_ui_shown: &'a mut HashSet<String>,
    pub(crate) duplicate_forced_reply_nudged: &'a mut bool,
    pub(crate) tool_calls: &'a mut Vec<ToolCallSummary>,
    pub(crate) tool_exec_records: &'a mut HashMap<String, ToolExecRecord>,
    pub(crate) tool_catalog: &'a crate::agent::tool_catalog::ToolCatalog,
    pub(crate) configured_repos: &'a [String],
    pub(crate) user_task: &'a str,
    pub(crate) discovery: Arc<Mutex<ChatDiscoveryState>>,
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

pub(crate) fn push_native_assistant_tool_calls(
    messages: &mut Vec<LlmTurnMessage>,
    step: &ChatAgentStep,
) {
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

pub(crate) fn record_session_file_edit(
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

pub(crate) async fn store_update_session_runtime(
    store: &dyn Store,
    session: &crate::store::ChatSession,
) -> Result<()> {
    store.update_chat_session(session).await
}

#[allow(clippy::too_many_arguments)]
async fn handle_llm_review_rejection(
    store: &dyn Store,
    session_id: &Uuid,
    workspace: &std::path::Path,
    config: &Config,
    store_arc: &Arc<dyn Store>,
    github: &Arc<GithubHarness>,
    mcp: &Arc<McpPool>,
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
        maybe_auto_approve_mutations(config, store_arc, github, mcp, tool_name, &queued).await?
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
            None,
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
        None,
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

enum LlmReviewRejectionOutcome {
    AutoApproved { detail: String },
    AwaitingApproval,
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
    fn tool_failure_nudge_uses_schema_when_args_incomplete() {
        let catalog = tool_catalog::ToolCatalog::new();
        let args = json!({ "repo": "acme/widget" });
        let msg = catalog.format_tool_failure_nudge(
            "pr_get_overview",
            &args,
            "failed to get pull request",
            &["acme/widget".into()],
        );
        assert!(msg.contains("pr_number"));
        assert!(msg.contains("[Harness]"));
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
        let snap = build_context_snapshot(&msgs, 1, &budget, &[], &[], None, &HashMap::new());
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
        let snap = build_context_snapshot(&msgs, 3, &budget, &[], &[], None, &HashMap::new());
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
        let snap = build_context_snapshot(&msgs, 2, &budget, &[], &[], None, &HashMap::new());
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
            &HashMap::new(),
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
            description: "Classify CI failures".into(),
            body: "## Rules\n- classify CI".into(),
            skill_refs: vec!["github-ops-tone".into()],
            tool_refs: vec!["pr_get_ci_snapshot".into()],
            always_load: true,
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
            &HashMap::new(),
        );
        assert_eq!(snap.skill_blocks.len(), 1);
        assert!(snap.skill_blocks[0].body.contains("classify CI"));
        // Frontmatter metadata is surfaced for the context-panel preview.
        assert_eq!(snap.skill_blocks[0].description, "Classify CI failures");
        assert!(snap.skill_blocks[0].always);
        assert_eq!(snap.skill_blocks[0].skills, vec!["github-ops-tone"]);
        assert_eq!(snap.skill_blocks[0].tools, vec!["pr_get_ci_snapshot"]);
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
        let snap = build_context_snapshot(&msgs, 1, &budget, &[], &[], None, &HashMap::new());
        assert_eq!(snap.messages[0].content.len(), body.len());
        assert_eq!(snap.messages[0].content, body);
    }

    #[test]
    fn build_context_snapshot_includes_trim_metadata() {
        use crate::llm::LlmTurnMessage;
        let budget = TokenBudget::from_config(64_000);
        let msgs = vec![
            LlmTurnMessage::new("system", "sys"),
            LlmTurnMessage::new("user", "[earlier context summary]\n- user asked about CI"),
            LlmTurnMessage::new("user", "latest question"),
        ];
        let snap = build_context_snapshot(&msgs, 2, &budget, &[], &[], None, &HashMap::new());
        assert_eq!(snap.context_trimmed_turns, 0);
        assert_eq!(
            snap.context_summary_note.as_deref(),
            Some("earlier turns summarized")
        );
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
        let snap = build_context_snapshot(&msgs, 1, &budget, &[], &[], Some((full, 2)), &HashMap::new());
        assert_eq!(snap.runtime_context_revision, Some(2));
        assert!(snap.messages[0].content.contains("Local store snapshot"));
        assert!(!snap.messages[0].content.contains("Store updates"));
    }

    #[test]
    fn build_context_snapshot_attaches_reasoning_original() {
        use crate::llm::LlmTurnMessage;
        let budget = TokenBudget::from_config(64_000);
        let content = "[agent reasoning summary]\n\n- bullet summary";
        let msgs = vec![LlmTurnMessage::new("user", content)];
        let mut originals = HashMap::new();
        originals.insert(content.to_string(), "full raw thinking trace".into());
        let snap = build_context_snapshot(&msgs, 1, &budget, &[], &[], None, &originals);
        assert_eq!(snap.messages[0].display_role, "reasoning");
        assert_eq!(snap.messages[0].content, "- bullet summary");
        assert_eq!(
            snap.messages[0].reasoning_original.as_deref(),
            Some("full raw thinking trace")
        );
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
    fn flow_activity_tools_are_transient() {
        assert!(!is_flow_activity_tool("skill_load"));
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
            original: None,
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
    fn pr_fetch_tools_allow_repeat_even_when_record_succeeded() {
        let args = json!({"repo": "acme/widget", "pr_number": 42});
        let fp = tool_call_fingerprint("pr_get_diff", &args);
        let mut records = HashMap::new();
        records.insert(
            fp,
            ToolExecRecord {
                succeeded: true,
                fail_count: 0,
            },
        );
        assert!(tool_allows_repeat_fetch("pr_get_diff"));
        assert!(!tool_allows_repeat_fetch("python_run"));
    }

    #[test]
    fn turn_start_tool_exec_records_are_empty_not_rehydrated() {
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
            reasoning_original: None,
        };
        let from_history = rehydrate_tool_exec_records_from_messages(&[msg]);
        let fp = tool_call_fingerprint("python_run", &args);
        assert_eq!(
            duplicate_tool_block_reason(from_history.get(&fp)),
            Some(DuplicateToolBlock::AlreadySucceeded)
        );
        let turn_start: HashMap<String, ToolExecRecord> = HashMap::new();
        assert_eq!(duplicate_tool_block_reason(turn_start.get(&fp)), None);
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
            reasoning_original: None,
        };
        let records = rehydrate_tool_exec_records_from_messages(&[msg]);
        let fp = tool_call_fingerprint("python_run", &args);
        assert_eq!(
            duplicate_tool_block_reason(records.get(&fp)),
            Some(DuplicateToolBlock::AlreadySucceeded)
        );
    }
}
