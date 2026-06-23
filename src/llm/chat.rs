use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{CoworkerError, Result};
use crate::llm::client::{strip_template_tokens, LlmClient};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatAgentAction {
    Reply,
    Tool,
}

#[derive(Debug, Clone)]
pub struct ResolvedToolCall {
    pub id: String,
    pub name: String,
    pub args: Value,
}

#[derive(Debug, Clone)]
pub struct ChatAgentStep {
    pub action: ChatAgentAction,
    pub message: String,
    pub tool_calls: Vec<ResolvedToolCall>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

#[derive(Debug, Clone, Default)]
pub struct ChatToolsTurn {
    pub content: String,
    /// Internal reasoning trace (`reasoning_content` / Ollama `thinking`), not the user reply.
    pub reasoning: String,
    pub tool_calls: Vec<LlmToolCall>,
}

#[derive(Debug, Clone)]
pub struct LlmTurnMessage {
    pub role: &'static str,
    pub content: String,
    pub tool_calls: Option<Vec<LlmToolCall>>,
    /// Set on `role: "tool"` result messages.
    pub tool_name: Option<String>,
    /// Native API tool call id matching the assistant `tool_calls[].id`.
    pub tool_call_id: Option<String>,
}

impl LlmTurnMessage {
    pub fn new(role: &'static str, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
            tool_calls: None,
            tool_name: None,
            tool_call_id: None,
        }
    }

    pub fn tool_result(tool_name: impl Into<String>, content: impl Into<String>) -> Self {
        Self::tool_result_with_id(None, tool_name, content)
    }

    pub fn tool_result_with_id(
        tool_call_id: Option<String>,
        tool_name: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            role: "tool",
            content: content.into(),
            tool_calls: None,
            tool_name: Some(tool_name.into()),
            tool_call_id,
        }
    }

    pub fn assistant_tool_call(content: impl Into<String>, tool_calls: Vec<LlmToolCall>) -> Self {
        Self {
            role: "assistant",
            content: content.into(),
            tool_calls: Some(tool_calls),
            tool_name: None,
            tool_call_id: None,
        }
    }
}

/// Serialize chat turns for Ollama native `/api/chat` (incl. `tool_name` on tool results).
pub fn llm_messages_to_api_value(messages: &[LlmTurnMessage]) -> Value {
    messages
        .iter()
        .map(ollama_api_message)
        .collect::<Vec<_>>()
        .into()
}

/// Serialize chat turns for OpenAI-compatible `/v1/chat/completions` (oMLX, vLLM, etc.).
pub fn llm_messages_to_openai_api_value(messages: &[LlmTurnMessage]) -> Value {
    messages
        .iter()
        .map(openai_api_message)
        .collect::<Vec<_>>()
        .into()
}

fn ollama_api_message(m: &LlmTurnMessage) -> Value {
    let mut obj = serde_json::Map::new();
    obj.insert("role".into(), Value::String(m.role.to_string()));
    obj.insert("content".into(), Value::String(m.content.clone()));
    if let Some(name) = &m.tool_name {
        obj.insert("tool_name".into(), Value::String(name.clone()));
    }
    if let Some(id) = &m.tool_call_id {
        obj.insert("tool_call_id".into(), Value::String(id.clone()));
    }
    if let Some(calls) = &m.tool_calls {
        let api_calls: Vec<Value> = calls
            .iter()
            .map(|tc| {
                serde_json::json!({
                    "id": tc.id,
                    "type": "function",
                    "function": {
                        "name": tc.name,
                        "arguments": tc.arguments,
                    }
                })
            })
            .collect();
        obj.insert("tool_calls".into(), Value::Array(api_calls));
    }
    Value::Object(obj)
}

fn openai_api_message(m: &LlmTurnMessage) -> Value {
    let mut obj = serde_json::Map::new();
    obj.insert("role".into(), Value::String(m.role.to_string()));

    if m.role == "tool" {
        obj.insert("content".into(), Value::String(m.content.clone()));
        if let Some(id) = &m.tool_call_id {
            obj.insert("tool_call_id".into(), Value::String(id.clone()));
        }
        return Value::Object(obj);
    }

    if let Some(calls) = &m.tool_calls {
        if m.content.trim().is_empty() {
            obj.insert("content".into(), Value::Null);
        } else {
            obj.insert("content".into(), Value::String(m.content.clone()));
        }
        let api_calls: Vec<Value> = calls
            .iter()
            .map(|tc| {
                serde_json::json!({
                    "id": tc.id,
                    "type": "function",
                    "function": {
                        "name": tc.name,
                        "arguments": openai_tool_arguments(&tc.arguments),
                    }
                })
            })
            .collect();
        obj.insert("tool_calls".into(), Value::Array(api_calls));
        return Value::Object(obj);
    }

    obj.insert("content".into(), Value::String(m.content.clone()));
    Value::Object(obj)
}

fn openai_tool_arguments(args: &Value) -> String {
    if let Some(s) = args.as_str() {
        s.to_string()
    } else {
        serde_json::to_string(args).unwrap_or_else(|_| "{}".to_string())
    }
}

/// Build a harness step from native `tool_calls` + optional assistant `content`.
pub fn step_from_native_turn(content: &str, tool_calls: &[LlmToolCall]) -> Result<ChatAgentStep> {
    if !tool_calls.is_empty() {
        let mut resolved = Vec::with_capacity(tool_calls.len());
        for call in tool_calls {
            let (name, args) = normalize_native_tool_call(&call.name, &call.arguments)?;
            resolved.push(ResolvedToolCall {
                id: call.id.clone(),
                name,
                args,
            });
        }
        if resolved.len() > 1 {
            tracing::debug!(
                "model returned {} tool_calls: {}",
                resolved.len(),
                resolved
                    .iter()
                    .map(|c| c.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        return Ok(ChatAgentStep {
            action: ChatAgentAction::Tool,
            message: strip_template_tokens(content),
            tool_calls: resolved,
        });
    }
    let message = strip_template_tokens(content).trim().to_string();
    if message.is_empty() {
        return Err(CoworkerError::Other(anyhow::anyhow!(
            "llm returned empty content and no tool_calls"
        )));
    }
    Ok(ChatAgentStep {
        action: ChatAgentAction::Reply,
        message,
        tool_calls: Vec::new(),
    })
}

fn normalize_native_tool_call(name: &str, args: &Value) -> Result<(String, Value)> {
    if name == "tool_call" {
        let inner = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| CoworkerError::Workflow("tool_call missing name".into()))?;
        let inner_args = args
            .get("args")
            .cloned()
            .unwrap_or_else(|| Value::Object(Default::default()));
        return Ok((inner.to_string(), inner_args));
    }
    Ok((name.to_string(), args.clone()))
}

pub fn tool_calls_stream_preview(tool_calls: &[LlmToolCall]) -> Option<String> {
    let call = tool_calls.first()?;
    let mut parts = Vec::new();
    if let Some(repo) = call.arguments.get("repo").and_then(|v| v.as_str()) {
        parts.push(format!("repo={repo}"));
    }
    if let Some(n) = call.arguments.get("pr_number").and_then(|v| v.as_u64()) {
        parts.push(format!("pr_number={n}"));
    }
    if let Some(n) = call.arguments.get("run_id").and_then(|v| v.as_i64()) {
        parts.push(format!("run_id={n}"));
    }
    let args_short = parts.join(", ");
    let first = if args_short.is_empty() {
        call.name.clone()
    } else {
        format!("{}({args_short})", call.name)
    };
    if tool_calls.len() == 1 {
        Some(first)
    } else {
        Some(format!("{first} +{} more", tool_calls.len() - 1))
    }
}

#[derive(Debug, Clone, Default)]
pub struct ChatStreamUpdate {
    /// Ollama `message.thinking` — internal reasoning, not the user-facing reply.
    pub reasoning: String,
    /// Partial assistant reply text while streaming.
    pub reply_partial: Option<String>,
    /// Partial native tool call label while streaming.
    pub tool_pending: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ChatStepOptions {
    pub compress_reasoning: bool,
    pub cancel: Option<Arc<AtomicBool>>,
}

impl Default for ChatStepOptions {
    fn default() -> Self {
        Self {
            compress_reasoning: true,
            cancel: None,
        }
    }
}

pub fn chat_cancelled_error() -> crate::error::CoworkerError {
    crate::error::CoworkerError::Workflow("chat cancelled".into())
}

pub fn chat_cancel_requested(cancel: &Option<Arc<AtomicBool>>) -> bool {
    cancel
        .as_ref()
        .is_some_and(|flag| flag.load(Ordering::Relaxed))
}

#[derive(Debug, Clone)]
pub struct ChatAgentStepOutcome {
    pub step: ChatAgentStep,
    /// Non-empty thinking trace to fold into LLM context (verbatim or summarized).
    pub reasoning_for_context: Option<String>,
}

impl LlmClient {
    pub async fn chat_agent_step_with_progress<F>(
        &self,
        messages: &[LlmTurnMessage],
        tools: &[Value],
        options: ChatStepOptions,
        mut on_stream: F,
    ) -> Result<ChatAgentStepOutcome>
    where
        F: FnMut(ChatStreamUpdate) + Send,
    {
        let msgs = if self.uses_ollama_native_chat() {
            llm_messages_to_api_value(messages)
        } else {
            llm_messages_to_openai_api_value(messages)
        };

        let mut last_reasoning = String::new();
        let turn = self
            .complete_chat_with_tools_with_progress(
                &msgs,
                tools,
                options.cancel.clone(),
                |content, thinking, tool_calls| {
                    if !thinking.is_empty() {
                        last_reasoning = thinking.to_string();
                    }
                    on_stream(ChatStreamUpdate {
                        reasoning: thinking.to_string(),
                        reply_partial: if tool_calls.is_empty() && !content.trim().is_empty() {
                            Some(content.to_string())
                        } else {
                            None
                        },
                        tool_pending: tool_calls_stream_preview(tool_calls),
                    });
                },
            )
            .await?;
        if last_reasoning.is_empty() && !turn.reasoning.trim().is_empty() {
            last_reasoning = turn.reasoning.clone();
        }
        let step = step_from_native_turn(&turn.content, &turn.tool_calls)?;
        let reasoning_for_context =
            if should_capture_reasoning_for_context(options.compress_reasoning, &last_reasoning) {
                Some(last_reasoning)
            } else {
                None
            };
        Ok(ChatAgentStepOutcome {
            step,
            reasoning_for_context,
        })
    }
}

/// Whether any non-empty thinking trace should be kept in LLM context.
pub fn should_capture_reasoning_for_context(enabled: bool, reasoning: &str) -> bool {
    enabled && !reasoning.trim().is_empty()
}

/// Whether to call the summarizer LLM (long traces only).
pub fn should_compress_reasoning(enabled: bool, reasoning: &str, min_chars: u32) -> bool {
    should_capture_reasoning_for_context(enabled, reasoning)
        && reasoning.trim().len() >= min_chars as usize
}

/// Verbatim or LLM-summarized body for `[agent reasoning summary]` context lines.
pub async fn materialize_reasoning_for_context(
    llm: &LlmClient,
    raw: &str,
    compress_enabled: bool,
    min_chars: u32,
) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(String::new());
    }
    if should_compress_reasoning(compress_enabled, raw, min_chars) {
        if let Some(summary) =
            maybe_compress_reasoning(llm, raw, compress_enabled, min_chars).await?
        {
            return Ok(summary);
        }
        tracing::warn!(
            reasoning_chars = trimmed.len(),
            "reasoning summarizer failed; keeping verbatim trace (no local truncation)"
        );
    }
    Ok(trimmed.to_string())
}

/// Summarize a long thinking trace via think=false LLM (used by `materialize_reasoning_for_context`).
async fn maybe_compress_reasoning(
    llm: &LlmClient,
    reasoning: &str,
    enabled: bool,
    min_chars: u32,
) -> Result<Option<String>> {
    if !should_compress_reasoning(enabled, reasoning, min_chars) {
        return Ok(None);
    }
    tracing::debug!(
        "compressing reasoning ~{} chars via summarizer",
        reasoning.len()
    );
    const MAX_ATTEMPTS: u32 = 3;
    for attempt in 1..=MAX_ATTEMPTS {
        match tokio::time::timeout(
            std::time::Duration::from_secs(90),
            llm.summarize_reasoning_trace(reasoning),
        )
        .await
        {
            Ok(Ok(summary)) if !summary.trim().is_empty() => {
                return Ok(Some(summary));
            }
            Ok(Ok(_)) => {
                tracing::warn!("reasoning compress returned empty (attempt {attempt})");
            }
            Ok(Err(e)) => {
                tracing::warn!("reasoning compress failed (attempt {attempt}): {e}");
            }
            Err(_) => {
                tracing::warn!("reasoning compress timed out after 90s (attempt {attempt})");
            }
        }
    }
    Ok(None)
}

/// Hoist `params` / `args` / JSON-string blobs into a flat tool_args object.
pub fn flatten_tool_args(value: &mut Value) {
    loop {
        if let Some(s) = value.as_str() {
            let trimmed = s.trim();
            if let Ok(parsed) = serde_json::from_str::<Value>(trimmed) {
                *value = parsed;
                continue;
            }
            break;
        }
        let Some(map) = value.as_object_mut() else {
            break;
        };
        let mut merged = false;
        for key in ["params", "args", "parameters"] {
            if let Some(inner) = map.remove(key).and_then(|v| v.as_object().cloned()) {
                for (k, v) in inner {
                    map.entry(k).or_insert(v);
                }
                merged = true;
            }
        }
        if !merged {
            break;
        }
    }
}

/// One MCP tool identifier (snake_case). Rejects pasted comma/space-separated lists.
pub fn is_plausible_tool_name(name: &str) -> bool {
    crate::agent::tool_catalog::is_plausible_tool_name(name)
}

/// Model returned action:reply but message is a plan ("I will investigate…") not the final answer.
pub fn reply_looks_like_planning(message: &str) -> bool {
    let lower = message.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return false;
    }
    const PHRASES: &[&str] = &[
        "i will investigate",
        "i'll investigate",
        "let me investigate",
        "i will look at",
        "i'll look at",
        "let me look at",
        "let's look at",
        "i will check",
        "i'll check",
        "let me check",
        "i need to investigate",
        "i will analyze",
        "i'll analyze",
        "next, i will",
        "next i will",
        "i will now investigate",
        "i'll now investigate",
        "is my choice. i will",
        "is my choice, i will",
        "i will rewrite",
        "i'll rewrite",
        "i will create",
        "i'll create",
        "i will build",
        "i'll build",
        "i will implement",
        "i'll implement",
        "i'll start by",
        "i will start by",
        "let me start by",
        "starting with the",
        "### plan",
        "implementation plan",
        "the implementation plan",
    ];
    PHRASES.iter().any(|phrase| lower.contains(phrase))
}

/// Model pasted `<tool_code>` / Python instead of native `tool_calls`.
pub fn reply_contains_fake_tool_invocation(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("<tool_code>")
        || lower.contains("</tool_code>")
        || lower.contains("import subprocess")
        || lower.contains("subprocess.run")
        || lower.contains("def write_file_bash")
        || (lower.contains("import os") && lower.contains("makedirs"))
}

/// User asked to build or change code/files in the workspace.
pub fn user_implies_implementation(user_message: &str) -> bool {
    let lower = user_message.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return false;
    }
    const EN: &[&str] = &[
        "implement",
        "create ",
        "rewrite",
        "build ",
        "write a",
        "write the",
        "start implementing",
        "scaffold",
        "add a",
        "set up",
        "setup ",
    ];
    if EN.iter().any(|p| lower.contains(p)) {
        return true;
    }
    user_message.contains("实现")
        || user_message.contains("写")
        || user_message.contains("创建")
        || user_message.contains("改成")
        || user_message.contains("重新")
        || user_message.contains("开始")
}

/// Reply claims files/code exist without a tool result in this turn.
pub fn reply_claims_implementation_done(message: &str) -> bool {
    let lower = message.trim().to_ascii_lowercase();
    const PHRASES: &[&str] = &[
        "i have created",
        "i've created",
        "files have been created",
        "following files have been created",
        "the following files have been created",
        "all files created",
        "have been created in",
        "have been created inside",
        "project structure",
    ];
    PHRASES.iter().any(|p| lower.contains(p)) || message.contains("✅")
}

/// Model deflects without calling change tools — often hallucinated paths follow.
pub fn reply_claims_cannot_see_changes(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    message.contains("系统限制")
        || message.contains("无法直接查看")
        || message.contains("无法查看")
        || message.contains("没有摘要信息")
        || lower.contains("system limitation")
        || lower.contains("cannot view") && lower.contains("detail")
        || lower.contains("unable to") && lower.contains("see")
}

/// Reply ends the turn too early — plan/fake tools/claimed work without tool_calls this turn.
pub fn reply_premature_for_task(message: &str, user_message: &str, tool_names: &[&str]) -> bool {
    if reply_looks_like_planning(message)
        || reply_claims_cannot_see_changes(message)
        || reply_contains_fake_tool_invocation(message)
    {
        return true;
    }
    if tool_names.is_empty() && user_implies_implementation(user_message) {
        return reply_claims_implementation_done(message) || reply_looks_like_planning(message);
    }
    false
}

/// Harness nudge when a premature reply is rejected before ending the turn.
pub fn reply_premature_nudge(message: &str, user_task: &str) -> String {
    if reply_contains_fake_tool_invocation(message) {
        return format!(
            "Your reply embedded <tool_code> or simulated Python/shell instead of native tool_calls. \
User asked: \"{user_task}\"\n\
Call write_file, bash_run, python_run, or edit_file via the native tool API — not prose scripts. \
After tool results are in context, reply in natural language."
        );
    }
    if reply_claims_cannot_see_changes(message) {
        return format!(
            "You replied without file/diff data. User asked: \"{user_task}\"\n\
pr_list_changed_files or pr_get_diff may help if change detail is needed."
        );
    }
    if reply_claims_implementation_done(message) {
        return format!(
            "You claimed files were created but no tool ran this turn. User asked: \"{user_task}\"\n\
Call write_file / bash_run / python_run via native tool_calls first, then summarize tool output."
        );
    }
    format!(
        "Your reply looked like a plan or incomplete answer. User asked: \"{user_task}\"\n\
Call tools via the native tool API before replying."
    )
}

/// User-facing reply ends mid-thought (model stopped early or ran out of tokens).
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openai_api_message_serializes_tool_arguments_as_json_string() {
        let msgs = vec![LlmTurnMessage::assistant_tool_call(
            String::new(),
            vec![LlmToolCall {
                id: "call_1".into(),
                name: "skill_load".into(),
                arguments: serde_json::json!({"name": "pr-review"}),
            }],
        )];
        let v = llm_messages_to_openai_api_value(&msgs);
        let args = v[0]["tool_calls"][0]["function"]["arguments"]
            .as_str()
            .unwrap();
        assert_eq!(args, r#"{"name":"pr-review"}"#);
    }

    #[test]
    fn openai_api_tool_result_omits_tool_name() {
        let msgs = vec![LlmTurnMessage::tool_result_with_id(
            Some("call_1".into()),
            "pr_get_overview",
            "overview body",
        )];
        let v = llm_messages_to_openai_api_value(&msgs);
        assert_eq!(v[0]["role"], "tool");
        assert_eq!(v[0]["tool_call_id"], "call_1");
        assert!(v[0].get("tool_name").is_none());
    }

    #[test]
    fn step_from_native_tool_call() {
        let calls = vec![LlmToolCall {
            id: "call_1".into(),
            name: "pr_list_open".into(),
            arguments: serde_json::json!({"repo": "acme/widget"}),
        }];
        let step = step_from_native_turn("", &calls).unwrap();
        assert_eq!(step.action, ChatAgentAction::Tool);
        assert_eq!(step.tool_calls.len(), 1);
        assert_eq!(step.tool_calls[0].name, "pr_list_open");
        assert_eq!(step.tool_calls[0].args["repo"], "acme/widget");
    }

    #[test]
    fn step_from_native_multiple_tool_calls() {
        let calls = vec![
            LlmToolCall {
                id: "call_1".into(),
                name: "pr_get_overview".into(),
                arguments: serde_json::json!({"repo": "o/r", "pr_number": 1}),
            },
            LlmToolCall {
                id: "call_2".into(),
                name: "pr_list_changed_files".into(),
                arguments: serde_json::json!({"repo": "o/r", "pr_number": 1}),
            },
        ];
        let step = step_from_native_turn("", &calls).unwrap();
        assert_eq!(step.tool_calls.len(), 2);
        assert_eq!(step.tool_calls[1].name, "pr_list_changed_files");
    }

    #[test]
    fn step_from_native_reply() {
        let step = step_from_native_turn("Hello from the model.", &[]).unwrap();
        assert_eq!(step.action, ChatAgentAction::Reply);
        assert_eq!(step.message, "Hello from the model.");
    }

    #[test]
    fn step_from_native_unwraps_tool_call_meta() {
        let calls = vec![LlmToolCall {
            id: "call_1".into(),
            name: "tool_call".into(),
            arguments: serde_json::json!({
                "name": "pr_get_overview",
                "args": {"repo": "o/r", "pr_number": 1}
            }),
        }];
        let step = step_from_native_turn("", &calls).unwrap();
        assert_eq!(step.tool_calls.len(), 1);
        assert_eq!(step.tool_calls[0].name, "pr_get_overview");
        assert_eq!(step.tool_calls[0].args["pr_number"], 1);
    }

    #[test]
    fn planning_reply_detected() {
        let msg = "Since I cannot access #19242, I will investigate #19238.";
        assert!(reply_looks_like_planning(msg));
        assert!(reply_premature_for_task(
            msg,
            "tell me why CI fails",
            &["pr_get_overview"],
        ));
    }

    #[test]
    fn fake_tool_code_reply_rejected() {
        let msg = "I'll start by creating files.\n\n<tool_code>\nimport os\nos.makedirs(\"go-pro-server\")\n</tool_code>";
        assert!(reply_contains_fake_tool_invocation(msg));
        assert!(reply_premature_for_task(msg, "重新用golang实现", &[]));
        assert!(reply_premature_nudge(msg, "重新用golang实现").contains("tool_code"));
    }

    #[test]
    fn claimed_implementation_without_tools_rejected() {
        let msg = "I have created a Python web server using **Flask** inside the `tmp-web-server/` directory.";
        assert!(reply_claims_implementation_done(msg));
        assert!(reply_premature_for_task(msg, "用 python 写。", &[]));
    }

    #[test]
    fn implementation_reply_ok_after_tools() {
        let msg = "I have created a Python web server using Flask.";
        assert!(!reply_premature_for_task(
            msg,
            "用 python 写。",
            &["write_file", "bash_run"],
        ));
    }

    #[test]
    fn go_rewrite_plan_rejected() {
        let msg = "I will rewrite the web server using **Go** (Golang). I'll start by creating the directory.";
        assert!(reply_premature_for_task(msg, "改成用 go 语言实现。", &[]));
    }

    #[test]
    fn should_capture_reasoning_when_non_empty() {
        assert!(!should_capture_reasoning_for_context(true, ""));
        assert!(!should_capture_reasoning_for_context(true, "   "));
        assert!(should_capture_reasoning_for_context(true, "short trace"));
        assert!(!should_capture_reasoning_for_context(false, "short trace"));
    }

    #[test]
    fn should_compress_reasoning_threshold() {
        assert!(!should_compress_reasoning(true, "short", 480));
        assert!(should_compress_reasoning(true, &"x".repeat(500), 480));
        assert!(!should_compress_reasoning(false, &"x".repeat(500), 480));
    }

    #[test]
    fn short_reasoning_materializes_verbatim_without_llm() {
        let raw = "Checking PR #42 CI before calling pr_get_overview.";
        assert!(should_capture_reasoning_for_context(true, raw));
        assert!(!should_compress_reasoning(true, raw, 480));
        assert_eq!(raw.trim(), raw);
    }

    #[test]
    fn long_reasoning_fallback_is_verbatim_not_local_truncation() {
        let raw = "x".repeat(5_000);
        assert!(should_compress_reasoning(true, &raw, 480));
        // When summarizer fails, materialize keeps full trace — never 2000-char local cut.
        assert!(!raw.contains("[reasoning truncated locally"));
    }

    #[test]
    fn tool_calls_stream_preview_shows_batch_size() {
        let calls = vec![
            LlmToolCall {
                id: "1".into(),
                name: "pr_get_overview".into(),
                arguments: serde_json::json!({"repo": "o/r", "pr_number": 1}),
            },
            LlmToolCall {
                id: "2".into(),
                name: "pr_list_changed_files".into(),
                arguments: serde_json::json!({"repo": "o/r", "pr_number": 1}),
            },
        ];
        let preview = tool_calls_stream_preview(&calls).unwrap();
        assert!(preview.contains("+1 more"));
    }

    #[test]
    fn flatten_nested_params_into_top_level_tool_args() {
        let mut args = serde_json::json!({"params": {"repo": "o/r", "pr_number": 1}});
        flatten_tool_args(&mut args);
        assert_eq!(args["repo"], serde_json::json!("o/r"));
        assert_eq!(args["pr_number"], serde_json::json!(1));
    }
}
