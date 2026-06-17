use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Utc;
use futures_util::future::join_all;
use serde_json::{json, Value};
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::agent::budget::TokenBudget;
use crate::agent::context::{
    estimate_message_tokens, format_tool_approval_pending_message, format_tool_context_message,
    harness_nudge_base, history_token_budget, pack_session_history_with_llm,
    trim_llm_messages_with_llm, trim_system_content, truncate_chars,
};
use crate::agent::parse::parse_failing_runs;
use crate::agent::tool_catalog;
use crate::app::{append_audit, AppEvent};
use crate::config::Config;
use crate::engine::{
    approvals, compose_system_prompt, load_chat_prompt_bundle, load_tools_doc_with_preferred,
};
use crate::error::{CoworkerError, Result};
use crate::llm::chat::{
    reply_claims_cannot_see_changes, reply_premature_for_task, ChatAgentStep, LlmToolCall,
    ResolvedToolCall,
};
use crate::llm::{ChatAgentAction, ChatStepOptions, LlmClient, LlmTurnMessage};
use crate::mcp::helpers::lazy_tool;
use crate::mcp::McpClient;
use crate::store::{Approval, ApprovalKind, ApprovalStatus, ChatMessage, ChatRole, Store};

const MUTATING_TOOLS: &[&str] = &["ci_rerun_workflow", "pr_create_backport", "pr_post_comment"];

const CONTEXT_MESSAGE_MAX_CHARS: usize = 12_000;

#[derive(Debug, Clone)]
pub struct ContextLine {
    /// TUI label — may differ from the LLM API role (e.g. tool results are API `user`).
    pub display_role: String,
    pub content: String,
    pub tokens: u32,
}

#[derive(Debug, Clone)]
pub struct ContextSnapshot {
    pub turn: u32,
    /// Estimated tokens in the current LLM message list.
    pub tokens_used: u32,
    /// Input budget used by trim_llm_messages (context_limit minus reserves).
    pub input_budget: u32,
    /// Model context window from config (llm.context_limit).
    pub context_limit: u32,
    pub message_count: usize,
    pub messages: Vec<ContextLine>,
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
    },
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
        )
    }

    pub fn display_line(&self) -> String {
        match self {
            Self::ContextSnapshot(_) => String::new(),
            Self::TurnThinking { .. } => "  … thinking".into(),
            Self::AssistantPartial { .. } | Self::ReasoningPartial { .. } => String::new(),
            Self::ToolPending { .. } => String::new(),
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
            Self::ReasoningSummary { preview } => format!("  … reasoning: {preview}"),
        }
    }

    pub fn status_text(&self) -> String {
        match self {
            Self::ContextSnapshot(_) => String::new(),
            Self::ReasoningCompressing => "chat: summarizing reasoning…".into(),
            Self::TurnThinking { turn, elapsed_secs } => {
                format!("chat thinking (step {turn}, {elapsed_secs}s)…")
            }
            Self::AssistantPartial { .. } => "chat: streaming reply…".into(),
            Self::ReasoningPartial { .. } => "chat: reasoning…".into(),
            Self::ToolPending { label } => format!("chat: {label}…"),
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
    input_budget: u32,
    context_limit: u32,
) -> ContextSnapshot {
    let tokens_used = crate::agent::context::estimate_messages_tokens(messages);
    let lines: Vec<ContextLine> = messages
        .iter()
        .map(|m| {
            let display_body = crate::agent::context::format_llm_message_for_context_panel(m);
            let content = truncate_chars(&display_body, CONTEXT_MESSAGE_MAX_CHARS);
            ContextLine {
                display_role: context_display_role(m.role, &m.content),
                content,
                tokens: estimate_message_tokens(m),
            }
        })
        .collect();
    ContextSnapshot {
        turn,
        tokens_used,
        input_budget,
        context_limit,
        message_count: messages.len(),
        messages: lines,
    }
}

/// Human-readable role for the context panel (API role alone is misleading).
pub fn context_display_role(api_role: &str, content: &str) -> String {
    if api_role == "system" {
        return "system".into();
    }
    if api_role == "assistant" {
        return "assistant".into();
    }
    if api_role == "tool" {
        return "tool".into();
    }
    let trimmed = content.trim_start();
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

fn emit_context_snapshot(
    progress: &Option<broadcast::Sender<AppEvent>>,
    messages: &[LlmTurnMessage],
    turn: u32,
    token_budget: &TokenBudget,
) {
    emit_progress(
        progress,
        ChatProgress::ContextSnapshot(build_context_snapshot(
            messages,
            turn,
            token_budget.input_budget(),
            token_budget.context_limit,
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
    mcp: Arc<dyn McpClient>,
    llm: Arc<LlmClient>,
    session_id: Uuid,
    resume: ResumeChatAfterApproval,
    progress: Option<broadcast::Sender<AppEvent>>,
    cancel: Option<Arc<AtomicBool>>,
) -> Result<ChatTurnResult> {
    run_chat_turn(
        config,
        store,
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
    mcp: Arc<dyn McpClient>,
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
    if !mcp.is_available() {
        return Err(CoworkerError::Other(anyhow::anyhow!(
            "unistar-mcp unavailable — set mcp.command and GH_TOKEN"
        )));
    }

    let session = match session_id {
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

    let context = build_store_context(store.as_ref()).await?;
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
    let runtime_context = format!(
        "## Configured repos\n{}\n\n## Local store snapshot\n{}",
        config.repos.join(", "),
        context,
    );
    let skill_paths: Vec<_> = config
        .chat
        .skills
        .iter()
        .map(std::path::PathBuf::from)
        .collect();
    let tools_doc = load_tools_doc_with_preferred(&config.chat.preferred_tools)?;
    let prompt_bundle =
        load_chat_prompt_bundle(&config.chat.agent, &skill_paths, tools_doc, runtime_context)?;
    let tool_catalog = tool_catalog::ToolCatalog::new(&config.chat.preferred_tools);
    let native_tools = tool_catalog.native_tool_definitions();

    let token_budget = TokenBudget::from_config(config.llm.context_limit);
    let history_token_cap = history_token_budget(&token_budget, config.chat.history_tokens);

    let mut system_content = compose_system_prompt(&prompt_bundle);
    system_content.push_str(
        "\n\nUse the native tools exposed in the API when you need data. \
Reply in natural language when the answer is complete.",
    );
    trim_system_content(&mut system_content, token_budget.system_budget());

    let mut llm_messages = vec![LlmTurnMessage::new("system", system_content)];

    llm_messages.extend(
        pack_session_history_with_llm(
            &history,
            config.chat.history_messages as usize,
            history_token_cap,
            llm.as_ref(),
            config.chat.compress_history,
            config.chat.history_summary_min_tokens,
        )
        .await?,
    );
    prune_stale_missing_arg_nudges(&mut llm_messages);

    if let Some(resume) = resume {
        apply_approval_resolution(
            store.as_ref(),
            &session.id,
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
    let mut tool_exec_records: HashMap<String, ToolExecRecord> = HashMap::new();
    let mut duplicate_tool_nudges: HashMap<String, u32> = HashMap::new();
    let mut duplicate_ui_shown: HashSet<String> = HashSet::new();
    let mut duplicate_forced_reply_nudged = false;
    let mut harness_corrections = 0u32;
    let turn_started = Instant::now();
    let mut llm_rounds = 0u32;

    emit_context_snapshot(&progress, &llm_messages, 0, &token_budget);

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

        emit_progress(
            &progress,
            ChatProgress::TurnThinking {
                turn: llm_rounds,
                elapsed_secs: turn_started.elapsed().as_secs(),
            },
        );
        race_chat_cancel(
            cancel.clone(),
            trim_llm_messages_with_llm(
                &mut llm_messages,
                token_budget.input_budget(),
                llm.as_ref(),
                config.chat.compress_history,
                config.chat.history_summary_min_tokens,
            ),
        )
        .await??;
        emit_context_snapshot(&progress, &llm_messages, llm_rounds, &token_budget);
        tracing::debug!(
            "chat context ~{} tokens (budget {})",
            crate::agent::context::estimate_messages_tokens(&llm_messages),
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
            emit_context_snapshot(&progress, &llm_messages, llm_rounds, &token_budget);
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
                    let nudge = if reply_claims_cannot_see_changes(&message) {
                        format!(
                            "You replied without file/diff data. User asked: \"{user_task}\"\n\
                             pr_list_changed_files or pr_get_diff may help if change detail is needed."
                        )
                    } else {
                        format!(
                            "Your reply looked like a plan or incomplete answer. User asked: \"{user_task}\""
                        )
                    };
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
                emit_context_snapshot(&progress, &llm_messages, llm_rounds, &token_budget);
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

                let prepared: Vec<PreparedToolCall> =
                    step.tool_calls.iter().map(prepare_tool_call).collect();

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
                    };
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
                        mcp.clone(),
                        cancel.clone(),
                        progress.clone(),
                        readonly,
                    )
                    .await?;
                    for outcome in outcomes {
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
                        };
                        record_tool_outcome(&mut round, outcome).await?;
                    }
                    emit_context_snapshot(&progress, &llm_messages, llm_rounds, &token_budget);
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
                            session_id: &session.id,
                            step: &step,
                            config,
                            store_arc: &store,
                            mcp: &mcp,
                            progress: &progress,
                            llm_messages: &mut llm_messages,
                            tool_calls: &mut tool_calls,
                            llm_rounds,
                            token_budget: &token_budget,
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
    emit_context_snapshot(&progress, &llm_messages, llm_rounds, &token_budget);
    Ok(ChatTurnResult {
        session_id: session.id,
        assistant_message: fallback,
        tool_calls,
        awaiting_approval: false,
    })
}

async fn build_store_context(store: &dyn Store) -> Result<String> {
    let mut lines = Vec::new();
    if let Some(d) = store.latest_digest().await? {
        lines.push(format!(
            "Latest digest ({}): attention={} flaky={} policy={}",
            d.date, d.summary.needs_attention, d.summary.flaky_candidates, d.summary.policy_gates
        ));
    }
    let pending = store.list_pending_approvals().await?;
    if !pending.is_empty() {
        lines.push(format!("Pending approvals: {}", pending.len()));
    }
    let sessions = store.list_chat_sessions(5).await?;
    if !sessions.is_empty() {
        lines.push(format!(
            "Recent chat sessions: {}",
            sessions
                .iter()
                .map(|s| format!("{} ({})", s.id, s.title))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let alerts = store
        .list_main_alerts(crate::store::MainAlertQuery {
            repo: None,
            unacknowledged_only: true,
            since_hours: Some(48),
            limit: 5,
        })
        .await?;
    if !alerts.is_empty() {
        lines.push(format!("Unacknowledged main alerts: {}", alerts.len()));
    }
    if lines.is_empty() {
        Ok("(no digest or alerts yet)".into())
    } else {
        Ok(lines.join("\n"))
    }
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
        "ci_get_run_summary" | "ci_get_failed_logs" | "ci_rerun_workflow"
    ) {
        if let Some(v) = tool_args.get("run_id") {
            if let Some(n) = v.as_i64() {
                tool_args["run_id"] = json!(n);
            } else if let Some(s) = v.as_str().and_then(|s| s.trim().parse::<i64>().ok()) {
                tool_args["run_id"] = json!(s);
            }
        }
    }
}

fn normalize_model_tool_args(tool_name: &str, tool_args: &mut Value) {
    if tool_name == "tool_call" {
        normalize_meta_tool_call_args(tool_args);
    } else {
        crate::llm::chat::flatten_tool_args(tool_args);
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
    catalog: &tool_catalog::ToolCatalog<'_>,
    tool_name: &str,
    tool_args: &Value,
    body: &str,
    configured_repos: &[String],
    messages: &mut Vec<LlmTurnMessage>,
) -> String {
    let parsed_missing: Vec<String> = missing_params_from_tool_error(body)
        .into_iter()
        .filter(|field| !tool_catalog::ToolCatalog::tool_arg_field_satisfied(tool_args, field))
        .collect();
    let schema_missing = catalog.missing_required_fields(tool_name, tool_args);
    let nudge = if let Some(field) = parsed_missing.first() {
        catalog.format_tool_args_nudge(tool_name, field, None, None)
    } else if let Some(field) = schema_missing.first() {
        catalog.format_tool_args_nudge(tool_name, field, None, None)
    } else {
        catalog.format_tool_failure_nudge(tool_name, tool_args, body, configured_repos)
    };
    push_harness_nudge(messages, nudge)
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
    tool_catalog: &'a crate::agent::tool_catalog::ToolCatalog<'a>,
    configured_repos: &'a [String],
    user_task: &'a str,
}

#[derive(Debug, Clone)]
struct ReadonlyToolOutcome {
    call: PreparedToolCall,
    output: String,
    ok: bool,
}

enum MutatingToolOutcome {
    Continue,
    AwaitingApproval,
}

fn prepare_tool_call(call: &ResolvedToolCall) -> PreparedToolCall {
    let mut args = call.args.clone();
    normalize_model_tool_args(&call.name, &mut args);
    coerce_numeric_tool_args(&call.name, &mut args);
    normalize_pr_tool_args(&call.name, &mut args);
    fill_default_diff_max_bytes(&call.name, &mut args);
    normalize_pr_tool_args(&call.name, &mut args);
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
    tool_catalog: &crate::agent::tool_catalog::ToolCatalog<'_>,
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

async fn maybe_block_duplicate_tool_call(
    round: &mut ToolRoundState<'_>,
    call: &PreparedToolCall,
    block: DuplicateToolBlock,
) -> Result<bool> {
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

async fn execute_readonly_tools_parallel(
    store: Arc<dyn Store>,
    mcp: Arc<dyn McpClient>,
    cancel: Option<Arc<AtomicBool>>,
    progress: Option<broadcast::Sender<AppEvent>>,
    calls: Vec<PreparedToolCall>,
) -> Result<Vec<ReadonlyToolOutcome>> {
    let futures = calls.into_iter().map(|call| {
        let store = Arc::clone(&store);
        let mcp = Arc::clone(&mcp);
        let cancel = cancel.clone();
        let progress = progress.clone();
        async move {
            ensure_chat_not_cancelled(&cancel)?;
            let args_short = format_tool_args_short(&call.args);
            emit_progress(
                &progress,
                ChatProgress::ToolStart {
                    name: call.name.clone(),
                    args_short,
                },
            );
            let tool_start = Instant::now();
            let result = match race_chat_cancel(
                cancel.clone(),
                execute_readonly_tool(store.as_ref(), mcp.as_ref(), &call.name, call.args.clone()),
            )
            .await
            {
                Ok(r) => r,
                Err(e) => return Err(e),
            };
            let (output, ok) = match result {
                Ok(o) if tool_output_indicates_failure(&call.name, &o) => (o, false),
                Ok(o) => (o, true),
                Err(e) => (format!("tool error: {e}"), false),
            };
            let elapsed_ms = tool_start.elapsed().as_millis();
            let ctx = format_tool_context_message(&call.name, &call.args, ok, &output);
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
            Ok(ReadonlyToolOutcome { call, output, ok })
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
    step: &'a ChatAgentStep,
    config: &'a Config,
    store_arc: &'a Arc<dyn Store>,
    mcp: &'a Arc<dyn McpClient>,
    progress: &'a Option<broadcast::Sender<AppEvent>>,
    llm_messages: &'a mut Vec<LlmTurnMessage>,
    tool_calls: &'a mut Vec<ToolCallSummary>,
    llm_rounds: u32,
    token_budget: &'a TokenBudget,
}

async fn handle_mutating_tool_call(
    ctx: MutatingToolContext<'_>,
    call: &PreparedToolCall,
) -> Result<MutatingToolOutcome> {
    let tool_name = call.name.as_str();
    let tool_args = &call.args;
    let queued = queue_mutating_approval(ctx.store, tool_name, tool_args).await?;
    if let Some(detail) =
        maybe_auto_approve_mutations(ctx.config, ctx.store_arc, ctx.mcp, &queued).await?
    {
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
    );
    Ok(MutatingToolOutcome::AwaitingApproval)
}

async fn execute_readonly_tool(
    store: &dyn Store,
    mcp: &dyn McpClient,
    tool_name: &str,
    tool_args: Value,
) -> Result<String> {
    match tool_name {
        "store_get_latest_digest" => format_store_latest_digest(store).await,
        "tool_list" => lazy_tool(mcp, "tool_list", json!({})).await,
        "tool_describe" => {
            let name = tool_args
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| CoworkerError::Workflow("tool_describe needs name".into()))?;
            lazy_tool(mcp, "tool_describe", json!({ "name": name })).await
        }
        "tool_call" => {
            let name = tool_args
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| CoworkerError::Workflow("tool_call needs name".into()))?;
            let args = tool_args.get("args").cloned().unwrap_or_else(|| json!({}));
            if is_mutating_tool(name) {
                return Err(CoworkerError::Workflow(format!(
                    "{name} is mutating — use approval action"
                )));
            }
            lazy_tool(mcp, "tool_call", json!({ "name": name, "args": args })).await
        }
        other if is_mutating_tool(other) => Err(CoworkerError::Workflow(format!(
            "{other} is mutating — use approval action"
        ))),
        other => lazy_tool(mcp, other, tool_args).await,
    }
}

async fn format_store_latest_digest(store: &dyn Store) -> Result<String> {
    let mut lines = Vec::new();
    if let Some(d) = store.latest_digest().await? {
        lines.push(format!(
            "Latest digest ({}) — needs_attention={} ignorable={} flaky={} policy={} complete={}",
            d.date,
            d.summary.needs_attention,
            d.summary.ignorable,
            d.summary.flaky_candidates,
            d.summary.policy_gates,
            d.summary.complete
        ));
        if !d.body_md.is_empty() {
            let body = if d.body_md.chars().count() > 4000 {
                format!("{}…\n[truncated]", truncate_chars(&d.body_md, 4000))
            } else {
                d.body_md.clone()
            };
            lines.push(String::new());
            lines.push(body);
        }
    } else {
        lines.push("No digest stored yet — run daily-work or another workflow first.".into());
    }

    let pending = store.list_pending_approvals().await?;
    if !pending.is_empty() {
        lines.push(format!("\nPending approvals: {}", pending.len()));
        for a in pending.iter().take(5) {
            lines.push(format!("- {:?} {}", a.kind, a.description));
        }
    }

    Ok(lines.join("\n"))
}

async fn queue_mutating_approval(
    store: &dyn Store,
    tool_name: &str,
    args: &Value,
) -> Result<QueuedApproval> {
    let repo = args
        .get("repo")
        .and_then(|v| v.as_str())
        .map(sanitize_repo_string)
        .unwrap_or_else(|| "unknown/repo".to_string());

    let comment_body = args
        .get("body")
        .and_then(|v| v.as_str())
        .map(str::to_string);

    let (kind, description, pr_number, run_id, target_branch) = match tool_name {
        "ci_rerun_workflow" => {
            let run_id = args
                .get("run_id")
                .and_then(|v| v.as_i64())
                .ok_or_else(|| CoworkerError::Workflow("ci_rerun_workflow needs run_id".into()))?;
            (
                ApprovalKind::RerunFlaky,
                format!("Chat: rerun workflow run {run_id} on {repo}"),
                None,
                Some(run_id),
                None,
            )
        }
        "pr_create_backport" => {
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
                Some(pr_number),
                None,
                Some(target_branch),
            )
        }
        "pr_post_comment" => {
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
                Some(pr_number),
                None,
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
        comment_body,
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
    mcp: &Arc<dyn McpClient>,
    queued: &QueuedApproval,
) -> Result<Option<String>> {
    if !config.chat.auto_approve_mutations {
        return Ok(None);
    }
    match approvals::process_decision(Arc::clone(store), Arc::clone(mcp), &queued.id, true).await {
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
        assert!(!is_mutating_tool("pr_list_open"));
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
        let catalog = tool_catalog::ToolCatalog::full();
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
        assert!(body.contains("not a missing-parameter error"));
        assert!(!body.contains("is missing required `repo`"));
    }

    #[test]
    fn tool_failure_nudge_uses_schema_when_args_incomplete() {
        let catalog = tool_catalog::ToolCatalog::full();
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
    fn sanitize_repo_strips_display_prefix() {
        assert_eq!(sanitize_repo_string("repo=acme/widget"), "acme/widget");
        assert_eq!(sanitize_repo_string("  acme/widget  "), "acme/widget");
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
    fn format_tool_args_short_includes_fields() {
        let args = json!({
            "repo": "unistar-ai/unistar-coworker",
            "pr": 42,
            "extra": "x"
        });
        let short = format_tool_args_short(&args);
        assert!(short.contains("repo=unistar-ai/unistar-coworker"));
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
        let snap = build_context_snapshot(&msgs, 1, 40_000, 64_000);
        let assistant = snap.messages.last().expect("assistant line");
        assert_eq!(assistant.display_role, "assistant");
        assert!(assistant.content.contains("tool_call: ci_get_failed_logs"));
        assert!(assistant.tokens > 0);
    }

    #[test]
    fn build_context_snapshot_includes_final_assistant_reply() {
        use crate::llm::LlmTurnMessage;
        let msgs = vec![
            LlmTurnMessage::new("user", "What failed in CI?"),
            LlmTurnMessage::new("assistant", "Run 123 failed due to a DB auth error."),
        ];
        let snap = build_context_snapshot(&msgs, 3, 40_000, 64_000);
        let last = snap.messages.last().expect("assistant reply");
        assert_eq!(last.display_role, "assistant");
        assert!(last.content.contains("DB auth error"));
    }

    #[test]
    fn build_context_snapshot_counts_messages() {
        use crate::llm::LlmTurnMessage;
        let msgs = vec![
            LlmTurnMessage::new("system", "You are helpful."),
            LlmTurnMessage::tool_result("pr_list_open", "#1 title"),
        ];
        let snap = build_context_snapshot(&msgs, 2, 52_523, 64_000);
        assert_eq!(snap.message_count, 2);
        assert_eq!(snap.messages.len(), 2);
        assert_eq!(snap.turn, 2);
        assert_eq!(snap.context_limit, 64_000);
        assert_eq!(snap.input_budget, 52_523);
        assert!(snap.tokens_used > 0);
        assert_eq!(snap.messages[0].display_role, "system");
        assert_eq!(snap.messages[1].display_role, "tool");
        assert!(snap.messages[1].content.contains("#1 title"));
        assert!(snap.messages[1].tokens > 0);
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
        assert_eq!(context_display_role("user", "list open PRs"), "user");
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
            "repo": "unistar-ai/unistar-coworker",
            "pr_number": 42,
            "extra": "x"
        });
        let short = format_tool_args_short(&args);
        assert!(short.contains("repo=unistar-ai/unistar-coworker"));
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
}
