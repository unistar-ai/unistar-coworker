use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde_json::Value;

use crate::error::{CoworkerError, Result};
use crate::llm::client::{extract_json_object, strip_template_tokens, LlmClient};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatAgentAction {
    Reply,
    Tool,
    Approval,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatAgentStep {
    pub action: ChatAgentAction,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub tool_name: Option<String>,
    #[serde(default)]
    pub tool_args: Option<Value>,
    /// Set by harness when the model pasted a comma-separated tool list into `tool_name`.
    #[serde(skip)]
    pub tool_name_was_pasted_list: bool,
    /// Original `tool_name` string from the model before normalization.
    #[serde(skip)]
    pub raw_tool_name: Option<String>,
    /// Harness recovered args that were wrongly embedded in `tool_name`.
    #[serde(skip)]
    pub tool_name_had_salvaged_args: bool,
    /// Native API tool call id (Ollama/OpenAI `tool_calls[].id`).
    #[serde(skip)]
    pub tool_call_id: Option<String>,
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
    pub tool_calls: Vec<LlmToolCall>,
}

#[derive(Debug, Clone)]
pub struct LlmTurnMessage {
    pub role: &'static str,
    pub content: String,
    pub tool_calls: Option<Vec<LlmToolCall>>,
    /// Set on `role: "tool"` result messages.
    pub tool_name: Option<String>,
}

impl LlmTurnMessage {
    pub fn new(role: &'static str, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
            tool_calls: None,
            tool_name: None,
        }
    }

    pub fn tool_result(tool_name: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: "tool",
            content: content.into(),
            tool_calls: None,
            tool_name: Some(tool_name.into()),
        }
    }

    pub fn assistant_tool_call(
        content: impl Into<String>,
        tool_calls: Vec<LlmToolCall>,
    ) -> Self {
        Self {
            role: "assistant",
            content: content.into(),
            tool_calls: Some(tool_calls),
            tool_name: None,
        }
    }
}

/// Serialize chat turns for Ollama/OpenAI `/chat` APIs (incl. native tool messages).
pub fn llm_messages_to_api_value(messages: &[LlmTurnMessage]) -> Value {
    messages
        .iter()
        .map(|m| {
            let mut obj = serde_json::Map::new();
            obj.insert("role".into(), Value::String(m.role.to_string()));
            obj.insert("content".into(), Value::String(m.content.clone()));
            if let Some(name) = &m.tool_name {
                obj.insert("tool_name".into(), Value::String(name.clone()));
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
        })
        .collect::<Vec<_>>()
        .into()
}

/// Build a harness step from native `tool_calls` + optional assistant `content`.
pub fn step_from_native_turn(content: &str, tool_calls: &[LlmToolCall]) -> Result<ChatAgentStep> {
    if let Some(call) = tool_calls.first() {
        if tool_calls.len() > 1 {
            tracing::warn!(
                "model returned {} tool_calls; using first (`{}`)",
                tool_calls.len(),
                call.name
            );
        }
        let (tool_name, tool_args) = normalize_native_tool_call(&call.name, &call.arguments)?;
        return Ok(ChatAgentStep {
            action: ChatAgentAction::Tool,
            message: content.to_string(),
            tool_name: Some(tool_name),
            tool_args: Some(tool_args),
            tool_name_was_pasted_list: false,
            raw_tool_name: None,
            tool_name_had_salvaged_args: false,
            tool_call_id: Some(call.id.clone()),
        });
    }
    let message = content.trim();
    if message.is_empty() {
        return Err(CoworkerError::Other(anyhow::anyhow!(
            "llm returned empty content and no tool_calls"
        )));
    }
    Ok(ChatAgentStep {
        action: ChatAgentAction::Reply,
        message: message.to_string(),
        tool_name: None,
        tool_args: None,
        tool_name_was_pasted_list: false,
        raw_tool_name: None,
        tool_name_had_salvaged_args: false,
        tool_call_id: None,
    })
}

fn normalize_native_tool_call(name: &str, args: &Value) -> Result<(String, Value)> {
    if name == "tool_call" {
        let inner = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| CoworkerError::Workflow("tool_call missing name".into()))?;
        let inner_args = args.get("args").cloned().unwrap_or_else(|| Value::Object(Default::default()));
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
    if args_short.is_empty() {
        Some(call.name.clone())
    } else {
        Some(format!("{}({args_short})", call.name))
    }
}

#[derive(Debug, Clone, Default)]
pub struct ChatStreamUpdate {
    /// Ollama `message.thinking` — internal reasoning, not the user-facing reply.
    pub reasoning: String,
    /// Partial `message` field when `action` is `reply`.
    pub reply_partial: Option<String>,
    /// Partial tool call while `action` is `tool` (or `tool_name` is present).
    pub tool_pending: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ChatStepOptions {
    pub compress_reasoning: bool,
    pub reasoning_compress_min_chars: u32,
    pub cancel: Option<Arc<AtomicBool>>,
}

impl Default for ChatStepOptions {
    fn default() -> Self {
        Self {
            compress_reasoning: true,
            reasoning_compress_min_chars: 480,
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
    /// Raw thinking trace to compress into context (handled by harness after stream ends).
    pub reasoning_to_compress: Option<String>,
}

impl LlmClient {
    #[allow(dead_code)]
    pub async fn chat_agent_step(&self, messages: &[LlmTurnMessage], tools: &[Value]) -> Result<ChatAgentStep> {
        Ok(self
            .chat_agent_step_with_progress(messages, tools, ChatStepOptions::default(), |_| {})
            .await?
            .step)
    }

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
        let msgs = llm_messages_to_api_value(messages);

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
        let mut step = step_from_native_turn(&turn.content, &turn.tool_calls)?;
        if step.action == ChatAgentAction::Reply && reply_message_looks_truncated(&step.message) {
            step.message = format!(
                "{}\n\n_[Reply may be incomplete — ask me to continue if you need the rest.]_",
                step.message
            );
        }
        let reasoning_to_compress = if should_compress_reasoning(
            options.compress_reasoning,
            &last_reasoning,
            options.reasoning_compress_min_chars,
        ) {
            Some(last_reasoning)
        } else {
            None
        };
        Ok(ChatAgentStepOutcome {
            step,
            reasoning_to_compress,
        })
    }
}

/// Summarize a long thinking trace via think=false LLM (called by harness after stream ends).
pub async fn compress_reasoning_for_context(
    llm: &LlmClient,
    reasoning: &str,
) -> Result<Option<String>> {
    maybe_compress_reasoning(llm, reasoning, true, 0).await
}

pub fn should_compress_reasoning(enabled: bool, reasoning: &str, min_chars: u32) -> bool {
    enabled && reasoning.trim().len() >= min_chars as usize
}

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
    const MAX_ATTEMPTS: u32 = 2;
    for attempt in 1..=MAX_ATTEMPTS {
        match tokio::time::timeout(
            std::time::Duration::from_secs(60),
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
                tracing::warn!("reasoning compress timed out after 60s (attempt {attempt})");
            }
        }
    }
    Ok(None)
}

pub fn parse_chat_agent_step(content: &str) -> Result<ChatAgentStep> {
    let cleaned = strip_model_artifacts(content);

    for candidate in [cleaned.as_str(), strip_markdown_fence(&cleaned).as_str()] {
        if let Ok(step) = try_parse_chat_step(candidate) {
            return Ok(normalize_chat_step(step));
        }
        if let Some(json) = extract_json_object(candidate) {
            if let Ok(step) = try_parse_chat_step(&json) {
                return Ok(normalize_chat_step(step));
            }
        }
        if let Some(step) = salvage_truncated_chat_step(candidate) {
            return Ok(normalize_chat_step(step));
        }
        if let Some(step) = salvage_plain_prose_reply(candidate) {
            tracing::debug!(
                "salvaged plain prose as action:reply (~{} chars)",
                step.message.len()
            );
            return Ok(normalize_chat_step(step));
        }
    }

    Err(CoworkerError::Other(anyhow::anyhow!(
        "llm parse chat step json; raw={content}"
    )))
}

/// Thinking models often put the complete tool JSON in `message.thinking` while `message.content`
/// holds prose or an incomplete object. Pick the best parseable candidate from both fields.
pub fn select_best_chat_step_text(content: &str, thinking: &str) -> String {
    let content = strip_model_artifacts(content);
    let thinking = strip_model_artifacts(thinking);
    let mut best_score = -1i32;
    let mut best = String::new();

    for text in [&content, &thinking] {
        if text.trim().is_empty() {
            continue;
        }
        for candidate in chat_step_json_candidates(text) {
            let score = score_chat_step_candidate(&candidate);
            if score > best_score {
                best_score = score;
                best = candidate;
            }
        }
    }

    if best_score >= 0 {
        if best.trim() != content.trim() && !thinking.trim().is_empty() {
            tracing::info!(
                "chat step: selected richer JSON (score {best_score}) over message.content"
            );
        }
        return best;
    }
    if !content.trim().is_empty() {
        return content;
    }
    if let Some(json) = extract_json_object(thinking.trim()) {
        tracing::warn!("llm message.content empty; recovered JSON from message.thinking");
        return json;
    }
    if thinking.trim().contains('{') {
        tracing::warn!("llm message.content empty; using message.thinking as fallback");
        return thinking.trim().to_string();
    }
    content
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

fn chat_step_json_candidates(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut push = |s: &str| {
        let t = s.trim();
        if !t.is_empty() && !out.iter().any(|existing| existing == t) {
            out.push(t.to_string());
        }
    };
    push(text);
    let unfenced = strip_markdown_fence(text);
    if unfenced.as_str() != text.trim() {
        push(&unfenced);
    }
    if let Some(j) = extract_json_object(text) {
        push(&j);
    }
    if unfenced.as_str() != text.trim() {
        if let Some(j) = extract_json_object(&unfenced) {
            push(&j);
        }
    }
    out
}

fn score_chat_step_candidate(candidate: &str) -> i32 {
    match parse_chat_agent_step(candidate) {
        Ok(step) => score_chat_agent_step(&step),
        Err(_) => -1,
    }
}

fn score_chat_agent_step(step: &ChatAgentStep) -> i32 {
    match step.action {
        ChatAgentAction::Reply => {
            if step.message.trim().len() >= 24 {
                2
            } else {
                0
            }
        }
        ChatAgentAction::Approval => 25,
        ChatAgentAction::Tool => {
            let mut score = 20;
            let name = step.tool_name.as_deref().unwrap_or("").trim();
            if name.is_empty() {
                score -= 20;
            }
            match &step.tool_args {
                None => score -= 12,
                Some(Value::String(s)) if s.trim().is_empty() => score -= 12,
                Some(Value::Object(map)) => {
                    score += (map.len() as i32).min(8);
                    if tool_arg_nonempty_str(map, "repo") {
                        score += 20;
                    }
                    if map.contains_key("pr_number") || map.contains_key("pr") {
                        score += 12;
                    }
                    if map.contains_key("run_id") {
                        score += 12;
                    }
                }
                Some(_) => score += 2,
            }
            score
        }
    }
}

fn tool_arg_nonempty_str(map: &serde_json::Map<String, Value>, key: &str) -> bool {
    map.get(key)
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.trim().is_empty())
}

fn try_parse_chat_step(content: &str) -> std::result::Result<ChatAgentStep, serde_json::Error> {
    #[derive(Deserialize)]
    struct FlexibleChatStepRaw {
        #[serde(default)]
        action: Option<ChatAgentAction>,
        #[serde(default)]
        message: String,
        #[serde(default, alias = "tool")]
        tool_name: Option<String>,
        #[serde(
            default,
            alias = "tool_args",
            alias = "args",
            alias = "parameters",
            alias = "params"
        )]
        tool_args: Option<Value>,
    }

    let value: Value = serde_json::from_str(content.trim())?;
    if let Some(map) = value.as_object() {
        let tool_name = json_string_alias(map, &["tool_name", "tool"])?;
        let action = match map.get("action") {
            Some(v) => serde_json::from_value(v.clone())?,
            None if tool_name.is_some() => ChatAgentAction::Tool,
            None => ChatAgentAction::Reply,
        };
        let message = match map.get("message") {
            Some(v) => serde_json::from_value(v.clone())?,
            None => String::new(),
        };
        let tool_args = merged_tool_args_aliases(map, tool_name.as_deref(), action);
        return Ok(ChatAgentStep {
            action,
            message,
            tool_name,
            tool_args,
            tool_name_was_pasted_list: false,
            raw_tool_name: None,
            tool_name_had_salvaged_args: false,
            tool_call_id: None,
        });
    }

    let raw: FlexibleChatStepRaw = serde_json::from_value(value)?;
    let action = raw.action.unwrap_or_else(|| {
        if raw.tool_name.is_some() {
            ChatAgentAction::Tool
        } else {
            ChatAgentAction::Reply
        }
    });
    Ok(ChatAgentStep {
        action,
        message: raw.message,
        tool_name: raw.tool_name,
        tool_args: raw.tool_args,
        tool_name_was_pasted_list: false,
        raw_tool_name: None,
        tool_name_had_salvaged_args: false,
            tool_call_id: None,
    })
}

fn json_string_alias(
    map: &serde_json::Map<String, Value>,
    keys: &[&str],
) -> std::result::Result<Option<String>, serde_json::Error> {
    for key in keys {
        let Some(value) = map.get(*key) else {
            continue;
        };
        if value.is_null() {
            return Ok(None);
        }
        return serde_json::from_value(value.clone()).map(Some);
    }
    Ok(None)
}

fn merged_tool_args_aliases(
    map: &serde_json::Map<String, Value>,
    tool_name: Option<&str>,
    action: ChatAgentAction,
) -> Option<Value> {
    if tool_name == Some("tool_call") {
        return merged_meta_tool_call_args(map);
    }

    let mut merged: Option<Value> = None;
    for key in ["tool_args", "params", "args", "parameters"] {
        if let Some(value) = map.get(key) {
            merge_tool_args_value(&mut merged, value.clone());
        }
    }
    if matches!(action, ChatAgentAction::Tool | ChatAgentAction::Approval) || tool_name.is_some() {
        merge_top_level_tool_arg_fields(&mut merged, map);
    }
    merged
}

fn parse_stringified_json_value(value: &mut Value) {
    while let Some(s) = value.as_str() {
        let trimmed = s.trim();
        let Ok(parsed) = serde_json::from_str::<Value>(trimmed) else {
            break;
        };
        *value = parsed;
    }
}

fn merge_tool_args_value(merged: &mut Option<Value>, mut value: Value) {
    flatten_tool_args(&mut value);
    if merged.is_none() {
        *merged = Some(value);
        return;
    }

    if let Some(Value::Object(dst)) = merged.as_mut() {
        if let Value::Object(src) = &mut value {
            for (key, value) in std::mem::take(src) {
                dst.entry(key).or_insert(value);
            }
            return;
        }
    }

    if merged
        .as_ref()
        .and_then(Value::as_object)
        .is_some_and(|m| m.is_empty())
    {
        *merged = Some(value);
    }
}

fn merge_top_level_tool_arg_fields(
    merged: &mut Option<Value>,
    map: &serde_json::Map<String, Value>,
) {
    let mut loose = serde_json::Map::new();
    for (key, value) in map {
        if is_chat_step_control_key(key) || value.is_null() {
            continue;
        }
        loose.insert(key.clone(), value.clone());
    }
    if loose.is_empty() {
        return;
    }
    let args = merged.get_or_insert_with(|| Value::Object(serde_json::Map::new()));
    if let Some(dst) = args.as_object_mut() {
        for (key, value) in loose {
            dst.entry(key).or_insert(value);
        }
    }
}

fn merged_meta_tool_call_args(map: &serde_json::Map<String, Value>) -> Option<Value> {
    let mut out = serde_json::Map::new();
    if let Some(value) = map.get("tool_args") {
        let mut value = value.clone();
        parse_stringified_json_value(&mut value);
        if let Some(src) = value.as_object() {
            out.extend(src.clone());
        } else {
            return Some(value);
        }
    }

    for key in ["name", "args", "params", "parameters"] {
        if let Some(value) = map.get(key) {
            let dest = if matches!(key, "params" | "parameters") {
                "args"
            } else {
                key
            };
            out.entry(dest.to_string()).or_insert_with(|| value.clone());
        }
    }

    let mut loose_target_args = serde_json::Map::new();
    for (key, value) in map {
        if is_chat_step_control_key(key)
            || matches!(key.as_str(), "name" | "args" | "params" | "parameters")
            || value.is_null()
        {
            continue;
        }
        loose_target_args.insert(key.clone(), value.clone());
    }
    if !loose_target_args.is_empty() {
        merge_meta_tool_call_target_args(&mut out, loose_target_args);
    }

    if out.is_empty() {
        None
    } else {
        Some(Value::Object(out))
    }
}

fn merge_meta_tool_call_target_args(
    out: &mut serde_json::Map<String, Value>,
    loose_target_args: serde_json::Map<String, Value>,
) {
    let args = out
        .entry("args".to_string())
        .or_insert_with(|| Value::Object(serde_json::Map::new()));
    parse_stringified_json_value(args);
    if let Some(dst) = args.as_object_mut() {
        for (key, value) in loose_target_args {
            dst.entry(key).or_insert(value);
        }
    }
}

fn is_chat_step_control_key(key: &str) -> bool {
    matches!(
        key,
        "action" | "message" | "tool_name" | "tool" | "tool_args"
    )
}

/// Remove gemma / template tokens that trail valid JSON or leak into message fields.
fn strip_model_artifacts(s: &str) -> String {
    strip_template_tokens(s)
}

fn salvage_truncated_chat_step(content: &str) -> Option<ChatAgentStep> {
    let lower = content.to_ascii_lowercase();
    if !lower.contains("\"action\"") || !lower.contains("reply") {
        return None;
    }
    let message = extract_json_string_value(content, "message")?;
    Some(ChatAgentStep {
        action: ChatAgentAction::Reply,
        message,
        tool_name: None,
        tool_args: None,
        tool_name_was_pasted_list: false,
        raw_tool_name: None,
        tool_name_had_salvaged_args: false,
            tool_call_id: None,
    })
}

/// Model returned markdown/prose instead of JSON — treat as action:reply when unambiguous.
fn salvage_plain_prose_reply(content: &str) -> Option<ChatAgentStep> {
    let text = strip_markdown_fence(&strip_model_artifacts(content));
    let trimmed = text.trim();
    if trimmed.len() < 24 {
        return None;
    }
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        return None;
    }
    if trimmed.contains("\"action\"") {
        return None;
    }
    Some(ChatAgentStep {
        action: ChatAgentAction::Reply,
        message: trimmed.to_string(),
        tool_name: None,
        tool_args: None,
        tool_name_was_pasted_list: false,
        raw_tool_name: None,
        tool_name_had_salvaged_args: false,
            tool_call_id: None,
    })
}

/// True when the response is not a usable chat step (truncation, empty, or unparseable JSON).
pub fn chat_response_needs_json_retry(content: &str, done_reason: Option<&str>) -> bool {
    if content.trim().is_empty() {
        return true;
    }
    if chat_response_needs_retry(content, done_reason) {
        return true;
    }
    parse_chat_agent_step(content).is_err()
}

/// Best-effort partial `message` field while JSON is still streaming (reply actions only).
#[cfg(test)]
pub(crate) fn extract_streaming_chat_message(buffer: &str) -> Option<String> {
    if !buffer_has_reply_action(buffer) || buffer_indicates_tool(buffer) {
        return None;
    }
    extract_json_string_value(buffer, "message")
        .filter(|m| !m.is_empty())
        .filter(|m| !message_looks_like_tool_narration(m))
}

/// Label for in-progress tool JSON (`pr_get_overview(repo=…)`).
#[cfg(test)]
pub(crate) fn extract_streaming_tool_pending(buffer: &str) -> Option<String> {
    if !buffer_indicates_tool(buffer) {
        return None;
    }
    let name = extract_json_string_value(buffer, "tool_name")?;
    let args = extract_streaming_tool_args_short(buffer);
    if args.is_empty() {
        Some(name)
    } else {
        Some(format!("{name}({args})"))
    }
}

#[cfg(test)]
fn extract_streaming_tool_args_short(buffer: &str) -> String {
    let mut parts = Vec::new();
    if let Some(repo) = extract_json_string_value(buffer, "repo") {
        parts.push(format!("repo={repo}"));
    }
    if let Some(n) = extract_json_number_value(buffer, "pr_number") {
        parts.push(format!("pr_number={n}"));
    }
    if let Some(n) = extract_json_number_value(buffer, "run_id") {
        parts.push(format!("run_id={n}"));
    }
    parts.join(", ")
}

#[cfg(test)]
fn extract_json_number_value(content: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let key_idx = content.find(&needle)?;
    let mut rest = content[key_idx + needle.len()..].trim_start();
    if rest.starts_with(':') {
        rest = rest[1..].trim_start();
    }
    let num: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if num.is_empty() {
        None
    } else {
        Some(num)
    }
}

#[cfg(test)]
fn message_looks_like_tool_narration(message: &str) -> bool {
    let t = message.trim();
    let lower = t.to_ascii_lowercase();
    crate::agent::context::is_tool_result_transcript(t)
        || t.starts_with("AI Tool:")
        || lower.contains("\"tool_name\"")
        || (lower.contains("args:") && lower.contains("pr_"))
        || (lower.contains("tool:") && lower.contains("pr_"))
}

#[cfg(test)]
fn buffer_indicates_tool(buffer: &str) -> bool {
    if matches!(extract_streaming_action(buffer), Some("tool")) {
        return true;
    }
    buffer.contains("\"tool_name\"")
}

#[cfg(test)]
fn buffer_has_reply_action(buffer: &str) -> bool {
    matches!(extract_streaming_action(buffer), Some("reply"))
}

/// Read a partial or complete `"action":"…"` value from streaming JSON.
#[cfg(test)]
fn extract_streaming_action(buffer: &str) -> Option<&str> {
    let lower = buffer.to_ascii_lowercase();
    let key_idx = lower.find("\"action\"")?;
    let mut rest = buffer[key_idx + 8..].trim_start();
    if rest.starts_with(':') {
        rest = rest[1..].trim_start();
    }
    if !rest.starts_with('"') {
        return None;
    }
    let value = rest[1..].split('"').next()?;
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
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
    ];
    PHRASES.iter().any(|phrase| lower.contains(phrase))
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

/// Reply ends the turn too early — based on assistant message quality only, not user phrasing.
pub fn reply_premature_for_task(
    message: &str,
    _user_message: &str,
    _tool_names: &[&str],
    _investigation_pr: Option<u32>,
) -> bool {
    reply_looks_like_planning(message) || reply_claims_cannot_see_changes(message)
}

/// Read a JSON string value for `key`, including when the closing quote is missing (truncated).
fn extract_json_string_value(content: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let key_idx = content.find(&needle)?;
    let mut rest = content[key_idx + needle.len()..].trim_start();
    if rest.starts_with(':') {
        rest = rest[1..].trim_start();
    }
    if !rest.starts_with('"') {
        return None;
    }
    let mut out = String::new();
    let mut chars = rest[1..].chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => break,
            },
            '"' => return Some(out),
            _ => out.push(c),
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

fn normalize_chat_step(mut step: ChatAgentStep) -> ChatAgentStep {
    if matches!(
        step.tool_name.as_deref(),
        Some("") | Some("none") | Some("null")
    ) {
        step.tool_name = None;
    }
    if let Some(name) = step.tool_name.take() {
        step.raw_tool_name = Some(name.clone());
        let (name_for_normalize, salvaged_args, had_embedded_args) =
            match salvage_embedded_tool_args_in_name(&name) {
                Some((base, args)) => (base, Some(args), true),
                None => (name, None, false),
            };
        if let Some(args) = salvaged_args {
            merge_salvaged_tool_args(&mut step.tool_args, args);
        }
        let (mut normalized, pasted) = normalize_tool_name(&name_for_normalize);
        if let Some((base, pr)) =
            crate::agent::tool_catalog::salvage_hallucinated_tool_name(&normalized)
        {
            normalized = base;
            if let Some(pr) = pr {
                merge_inferred_pr_number(&mut step.tool_args, pr);
            }
        }
        if let Some((base, pr)) = split_pr_suffix_from_tool_name(&normalized) {
            normalized = base;
            merge_inferred_pr_number(&mut step.tool_args, pr);
        }
        step.tool_name = Some(normalized);
        step.tool_name_was_pasted_list = !had_embedded_args && pasted;
        step.tool_name_had_salvaged_args = had_embedded_args;
    }
    if !step.message.is_empty() {
        step.message = sanitize_chat_message(&step.message);
    }
    step
}

/// One MCP tool identifier (snake_case). Rejects pasted comma/space-separated lists.
pub fn is_plausible_tool_name(name: &str) -> bool {
    crate::agent::tool_catalog::is_plausible_tool_name(name)
}

/// If the model pasted the preferred-tools list into `tool_name`, keep the first valid name.
pub fn normalize_tool_name(raw: &str) -> (String, bool) {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return (String::new(), false);
    }
    if is_plausible_tool_name(trimmed) && !trimmed.contains(',') {
        return (trimmed.trim_matches('`').to_string(), false);
    }
    let pasted_list = trimmed.contains(',');
    let segments: Vec<&str> = if pasted_list {
        trimmed.split(',').collect()
    } else {
        trimmed.split_whitespace().collect()
    };
    let multi_segment = segments.len() > 1;
    for part in segments {
        let part = part.trim().trim_matches('`');
        if is_plausible_tool_name(part) {
            return (part.to_string(), pasted_list || multi_segment);
        }
    }
    (trimmed.to_string(), pasted_list)
}

/// `pr_get_overview, pr_number: 19277` or `pr_list_changed_files, tool_args: {...}` → salvaged args.
pub fn salvage_embedded_tool_args_in_name(raw: &str) -> Option<(String, Value)> {
    let trimmed = raw.trim();
    if let Some((tool, args)) = salvage_tool_args_blob_in_name(trimmed) {
        return Some((tool, args));
    }
    if !trimmed.contains(',') {
        return None;
    }
    let segments: Vec<&str> = trimmed.split(',').map(str::trim).collect();
    if segments.len() < 2 {
        return None;
    }
    let first = segments[0].trim_matches('`');
    if !is_plausible_tool_name(first) {
        return None;
    }
    for part in segments.iter().skip(1) {
        let part = part.trim().trim_matches('`');
        if is_plausible_tool_name(part) {
            return None;
        }
    }
    let mut map = serde_json::Map::new();
    for part in segments.iter().skip(1) {
        let part = part.trim();
        let lower = part.to_ascii_lowercase();
        if lower.starts_with("tool_args:") || lower.starts_with("tool_args=") {
            let blob = part
                .split_once(':')
                .or_else(|| part.split_once('='))?
                .1
                .trim();
            let obj = parse_embedded_tool_args_json_blob(blob)?;
            for (key, value) in obj.as_object()? {
                map.insert(key.clone(), value.clone());
            }
            continue;
        }
        let (key, value) = parse_loose_tool_arg_segment(part)?;
        map.insert(key.clone(), loose_tool_arg_value(&key, &value));
    }
    if map.is_empty() {
        return None;
    }
    Some((first.to_string(), Value::Object(map)))
}

fn salvage_tool_args_blob_in_name(trimmed: &str) -> Option<(String, Value)> {
    let comma = trimmed.find(',')?;
    let first = trimmed[..comma].trim().trim_matches('`');
    if !is_plausible_tool_name(first) {
        return None;
    }
    let rest = trimmed[comma + 1..].trim();
    let lower = rest.to_ascii_lowercase();
    if !lower.starts_with("tool_args:") && !lower.starts_with("tool_args=") {
        return None;
    }
    let blob = rest
        .split_once(':')
        .or_else(|| rest.split_once('='))?
        .1
        .trim();
    let args = parse_embedded_tool_args_json_blob(blob)?;
    Some((first.to_string(), args))
}

fn parse_embedded_tool_args_json_blob(blob: &str) -> Option<Value> {
    let blob = blob.trim();
    if blob.is_empty() {
        return None;
    }
    if let Ok(value) = serde_json::from_str::<Value>(blob) {
        if value.is_object() {
            return Some(value);
        }
    }
    let extracted = extract_json_object(blob)?;
    let value: Value = serde_json::from_str(&extracted).ok()?;
    value.is_object().then_some(value)
}

fn parse_loose_tool_arg_segment(segment: &str) -> Option<(String, String)> {
    let segment = segment.trim();
    let (key, value) = segment
        .split_once(':')
        .or_else(|| segment.split_once('='))?;
    let key = key
        .trim()
        .trim_matches(|c| c == '"' || c == '\'' || c == '`');
    let value = value
        .trim()
        .trim_matches(|c| c == '"' || c == '\'' || c == '`');
    if key.is_empty() || value.is_empty() {
        return None;
    }
    if !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    Some((key.to_string(), value.to_string()))
}

fn loose_tool_arg_value(key: &str, value: &str) -> Value {
    match key {
        "pr_number" | "run_id" | "limit" | "max_bytes" => value
            .parse::<u64>()
            .map(Value::from)
            .unwrap_or_else(|_| Value::String(value.to_string())),
        _ => Value::String(value.to_string()),
    }
}

fn merge_salvaged_tool_args(tool_args: &mut Option<Value>, salvaged: Value) {
    let args = tool_args.get_or_insert_with(|| Value::Object(serde_json::Map::new()));
    let Some(map) = args.as_object_mut() else {
        return;
    };
    let Some(salvaged_map) = salvaged.as_object() else {
        return;
    };
    for (key, value) in salvaged_map {
        map.entry(key.clone()).or_insert_with(|| value.clone());
    }
}

/// Example of a valid single-tool JSON object for harness nudges.
pub fn format_correct_tool_call_json(tool_name: &str, tool_args: Option<&Value>) -> String {
    let mut payload = serde_json::Map::new();
    payload.insert("action".into(), Value::String("tool".into()));
    payload.insert("tool_name".into(), Value::String(tool_name.to_string()));
    let args = tool_args
        .filter(|a| a.as_object().is_some_and(|m| !m.is_empty()))
        .cloned()
        .unwrap_or_else(|| {
            Value::Object(serde_json::Map::from_iter([
                ("repo".into(), Value::String("owner/repo".into())),
                ("pr_number".into(), Value::from(19277_u64)),
            ]))
        });
    payload.insert("tool_args".into(), args);
    serde_json::to_string_pretty(&Value::Object(payload))
        .unwrap_or_else(|_| format!(r#"{{"action":"tool","tool_name":"{tool_name}"}}"#))
}

/// Harness nudge when args were embedded in `tool_name` (salvaged and executed).
pub fn format_salvaged_tool_name_harness_message(
    executed_tool: &str,
    raw_tool_name: &str,
    tool_args: Option<&Value>,
) -> String {
    let correct = format_correct_tool_call_json(executed_tool, tool_args);
    format!(
        "Malformed tool call: parameters were inside tool_name instead of a separate tool_args \
         object. Only `{executed_tool}` was executed this turn (harness salvaged the args).\n\n\
         What you sent in tool_name:\n`{raw_tool_name}`\n\n\
         Correct format (one tool per JSON object):\n```json\n{correct}\n```\n\n\
         Close tool_name with `\"` immediately after the tool name; put every parameter in tool_args."
    )
}

/// Harness nudge when the model pasted multiple tool names into `tool_name`.
pub fn format_pasted_tool_names_harness_message(
    executed_tool: &str,
    raw_tool_name: &str,
    tool_args: Option<&Value>,
) -> String {
    let mut payload = serde_json::Map::new();
    payload.insert("action".into(), Value::String("tool".into()));
    payload.insert(
        "tool_name".into(),
        Value::String(raw_tool_name.trim().to_string()),
    );
    if let Some(args) = tool_args {
        if args.as_object().is_some_and(|m| !m.is_empty()) {
            payload.insert("tool_args".into(), args.clone());
        }
    }
    let input = serde_json::to_string_pretty(&Value::Object(payload))
        .unwrap_or_else(|_| format!(r#"{{"action":"tool","tool_name":"{raw_tool_name}"}}"#));
    let correct = format_correct_tool_call_json(executed_tool, tool_args);

    format!(
        "You pasted multiple tool names into tool_name. Only `{executed_tool}` \
         was executed this turn.\n\nYour tool call input:\n```json\n{input}\n```\n\n\
         Correct format (one tool per JSON object):\n```json\n{correct}\n```\n\n\
         Close tool_name with `\"` after the tool name; never put tool_args or parameters inside \
         the tool_name string."
    )
}

/// `pr_get_overview_19258` → (`pr_get_overview`, 19258) — models sometimes suffix `#N` onto the tool name.
fn split_pr_suffix_from_tool_name(name: &str) -> Option<(String, u32)> {
    let name = name.trim().trim_matches('`');
    let idx = name.rfind('_')?;
    if idx == 0 || idx + 1 >= name.len() {
        return None;
    }
    let suffix = &name[idx + 1..];
    if !suffix.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let pr: u32 = suffix.parse().ok()?;
    if pr == 0 {
        return None;
    }
    let base = &name[..idx];
    if !is_plausible_tool_name(base) || !tool_name_accepts_pr_suffix(base) {
        return None;
    }
    Some((base.to_string(), pr))
}

fn tool_name_accepts_pr_suffix(base: &str) -> bool {
    matches!(
        base,
        "pr_get_overview"
            | "pr_get_status"
            | "pr_get_merge_blockers"
            | "pr_list_changed_files"
            | "pr_get_diff"
            | "ci_analyze_pr_failures"
    )
}

fn merge_inferred_pr_number(tool_args: &mut Option<Value>, pr: u32) {
    let args = tool_args.get_or_insert_with(|| serde_json::json!({}));
    if let Some(map) = args.as_object_mut() {
        if map
            .get("pr_number")
            .and_then(|v| v.as_u64())
            .filter(|&n| n > 0)
            .is_none()
        {
            map.insert("pr_number".into(), serde_json::json!(pr));
        }
    }
}

/// Chat replies are user-facing prose — do not apply classify-style `thought:` truncation.
pub(crate) fn sanitize_chat_message(text: &str) -> String {
    let text = crate::llm::client::strip_template_tokens(text);
    if text.is_empty() {
        return String::new();
    }

    let mut cut_at = text.len();
    for marker in [
        "```json",
        "```JSON",
        "```",
        "<channel|>",
        "<|",
        "\n{\"action\"",
    ] {
        if let Some(idx) = text.find(marker) {
            cut_at = cut_at.min(idx);
        }
    }

    let mut s = text[..cut_at].trim().to_string();
    s = s.trim_end_matches(['\'', '"', '`', ',', ' ']).to_string();
    s.trim().to_string()
}

/// User-facing reply ends mid-thought despite valid JSON (model stopped early or ran out of tokens).
pub fn reply_message_looks_truncated(message: &str) -> bool {
    let t = message.trim();
    if t.len() < 48 {
        return false;
    }
    if reply_message_looks_complete(t) {
        return false;
    }
    if reply_message_has_broken_suffix(t) {
        return true;
    }
    if let Some(last_line) = t.lines().last().map(str::trim) {
        if last_line.len() > 24
            && !reply_message_looks_complete(last_line)
            && reply_message_has_broken_suffix(last_line)
        {
            return true;
        }
    }
    false
}

fn reply_message_looks_complete(s: &str) -> bool {
    s.ends_with(['.', '!', '?', ':', ')', ']', '*', '`'])
        || s.ends_with("```")
        || s.ends_with("...")
        || s.ends_with('…')
}

fn reply_message_has_broken_suffix(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    const SUFFIXES: &[&str] = &[
        " i",
        " a",
        " an",
        " the",
        " and",
        " or",
        " but",
        " to",
        " of",
        " for",
        " with",
        " however,",
        " however",
        " though",
        " because",
        " when",
        " that",
        " which",
        " it",
        " my",
        " your",
        " we",
        " they",
        " this",
        " these",
        " those",
        " if",
        " as",
    ];
    if SUFFIXES.iter().any(|suffix| lower.ends_with(suffix)) {
        return true;
    }
    s.split_whitespace()
        .last()
        .is_some_and(|word| word.len() <= 2)
}

/// True when the model hit output length or returned truncated JSON / reply prose.
pub fn chat_response_needs_retry(content: &str, done_reason: Option<&str>) -> bool {
    if done_reason == Some("length") || done_reason == Some("max_tokens") {
        return true;
    }
    if let Ok(step) = parse_chat_agent_step(content) {
        return step.action == ChatAgentAction::Reply
            && reply_message_looks_truncated(&step.message);
    }
    salvage_truncated_chat_step(content).is_some()
}

fn strip_markdown_fence(text: &str) -> String {
    let text = text.trim();
    if text.starts_with("```") {
        let inner = text.trim_start_matches('`');
        let inner = inner.trim_start_matches("json").trim_start();
        if let Some(end) = inner.rfind("```") {
            return inner[..end].trim().to_string();
        }
    }
    text.to_string()
}

pub fn chat_step_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "action": {
                "type": "string",
                "enum": ["reply", "tool", "approval"]
            },
            "message": { "type": "string" },
            "tool_name": { "type": "string" },
            "tool_args": { "type": "object" }
        },
        "required": ["action"],
        "additionalProperties": false
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_reply_step() {
        let step = parse_chat_agent_step(r#"{"action":"reply","message":"hello"}"#).unwrap();
        assert_eq!(step.action, ChatAgentAction::Reply);
        assert_eq!(step.message, "hello");
    }

    #[test]
    fn parse_tool_step_from_fence() {
        let raw = "```json\n{\"action\":\"tool\",\"tool_name\":\"pr_list_open\",\"tool_args\":{\"repo\":\"o/r\"}}\n```";
        let step = parse_chat_agent_step(raw).unwrap();
        assert_eq!(step.action, ChatAgentAction::Tool);
        assert_eq!(step.tool_name.as_deref(), Some("pr_list_open"));
    }

    #[test]
    fn parse_reply_strips_channel_token_from_message() {
        let raw = r#"{"action":"reply","message":"Hello capabilities:\n* item one\n<channel|>{"}"#;
        let step = parse_chat_agent_step(raw).unwrap();
        assert_eq!(step.action, ChatAgentAction::Reply);
        assert!(step.message.contains("item one"));
        assert!(!step.message.contains("<channel|>"));
        assert!(!step.message.contains('{'));
    }

    #[test]
    fn salvage_plain_prose_as_reply() {
        let raw = "I've identified **PR #19238** as a candidate where the CI failure is not an \
approval case. Would you like me to proceed with that?";
        let step = parse_chat_agent_step(raw).unwrap();
        assert_eq!(step.action, ChatAgentAction::Reply);
        assert!(step.message.contains("#19238"));
    }

    #[test]
    fn broken_json_still_fails_without_prose() {
        assert!(parse_chat_agent_step(r#"{"action":"reply","message":"hi"#).is_ok());
        assert!(parse_chat_agent_step(r#"{ "action": "#).is_err());
    }

    #[test]
    fn chat_response_needs_json_retry_on_broken_json() {
        let broken = r#"{ "action": "reply", "message": "#;
        assert!(chat_response_needs_json_retry(broken, Some("stop")));
        assert!(chat_response_needs_json_retry("", Some("stop")));
        assert!(!chat_response_needs_json_retry(
            r#"{"action":"reply","message":"Done."}"#,
            Some("stop")
        ));
    }

    #[test]
    fn salvage_truncated_reply_message() {
        let raw = r#"{"action":"reply","message":"Short answer without closing quote"#;
        let step = parse_chat_agent_step(raw).unwrap();
        assert_eq!(step.message, "Short answer without closing quote");
    }

    #[test]
    fn parse_tool_step_with_gemma_suffix_token() {
        let raw = r#"{
  "action": "tool",
  "tool_name": "pr_list_open"
  }
  <|tool_response|>"#;
        let step = parse_chat_agent_step(raw).unwrap();
        assert_eq!(step.action, ChatAgentAction::Tool);
        assert_eq!(step.tool_name.as_deref(), Some("pr_list_open"));
        assert!(step.tool_args.is_none());
    }

    #[test]
    fn parse_tool_step_normalizes_pasted_tool_list() {
        let raw = r#"{"action":"tool","tool_name":"pr_get_overview, pr_list_open, pr_get_diff"}"#;
        let step = parse_chat_agent_step(raw).unwrap();
        assert_eq!(step.action, ChatAgentAction::Tool);
        assert_eq!(step.tool_name.as_deref(), Some("pr_get_overview"));
        assert!(step.tool_name_was_pasted_list);
        assert_eq!(
            step.raw_tool_name.as_deref(),
            Some("pr_get_overview, pr_list_open, pr_get_diff")
        );
    }

    #[test]
    fn salvage_tool_args_blob_embedded_in_tool_name() {
        let raw =
            r#"pr_list_changed_files, tool_args: { "repo": "acme/widget", "pr_number": 19277 }"#;
        let (tool, args) = salvage_embedded_tool_args_in_name(raw).expect("salvaged");
        assert_eq!(tool, "pr_list_changed_files");
        assert_eq!(args["repo"], "acme/widget");
        assert_eq!(args["pr_number"], 19277_u64);
    }

    #[test]
    fn parse_tool_step_salvages_tool_args_blob_in_tool_name() {
        let raw = r#"{"action":"tool","tool_name":"pr_list_changed_files, tool_args: { \"repo\": \"acme/widget\", \"pr_number\": 19277 }"}"#;
        let step = parse_chat_agent_step(raw).unwrap();
        assert_eq!(step.tool_name.as_deref(), Some("pr_list_changed_files"));
        assert!(step.tool_name_had_salvaged_args);
        assert!(!step.tool_name_was_pasted_list);
        assert_eq!(
            step.tool_args.as_ref().and_then(|a| a.get("pr_number")),
            Some(&serde_json::json!(19277))
        );
        assert_eq!(
            step.tool_args
                .as_ref()
                .and_then(|a| a.get("repo"))
                .and_then(|v| v.as_str()),
            Some("acme/widget")
        );
    }

    #[test]
    fn salvaged_tool_name_harness_message_shows_correct_format() {
        let msg = format_salvaged_tool_name_harness_message(
            "pr_list_changed_files",
            r#"pr_list_changed_files, tool_args: { "repo": "acme/widget", "pr_number": 19277 }"#,
            Some(&serde_json::json!({"repo":"acme/widget","pr_number":19277})),
        );
        assert!(msg.contains("Malformed tool call"));
        assert!(msg.contains("Correct format"));
        assert!(msg.contains("\"tool_args\""));
        assert!(msg.contains("pr_list_changed_files"));
    }

    #[test]
    fn salvage_embedded_pr_number_from_tool_name() {
        let salvaged = salvage_embedded_tool_args_in_name("pr_get_overview, pr_number: 19277")
            .expect("salvaged");
        assert_eq!(salvaged.0, "pr_get_overview");
        assert_eq!(salvaged.1["pr_number"], serde_json::json!(19277));
    }

    #[test]
    fn parse_tool_step_salvages_args_leaked_into_tool_name() {
        let raw = r#"{"action":"tool","tool_name":"pr_get_overview, pr_number: 19277"}"#;
        let step = parse_chat_agent_step(raw).unwrap();
        assert_eq!(step.tool_name.as_deref(), Some("pr_get_overview"));
        assert!(!step.tool_name_was_pasted_list);
        assert_eq!(
            step.tool_args.as_ref().and_then(|a| a.get("pr_number")),
            Some(&serde_json::json!(19277))
        );
    }

    #[test]
    fn embedded_args_salvage_does_not_trigger_on_multi_tool_paste() {
        assert!(
            salvage_embedded_tool_args_in_name("pr_get_overview, pr_list_open, pr_get_diff")
                .is_none()
        );
        let raw = r#"{"action":"tool","tool_name":"pr_get_overview, pr_list_open, pr_get_diff"}"#;
        let step = parse_chat_agent_step(raw).unwrap();
        assert!(step.tool_name_was_pasted_list);
    }

    #[test]
    fn pasted_tool_names_harness_message_includes_input() {
        let msg = format_pasted_tool_names_harness_message(
            "pr_get_overview",
            "pr_get_overview, pr_list_open, pr_get_diff",
            Some(&serde_json::json!({"repo":"o/r","pr_number":1})),
        );
        assert!(msg.contains("pr_get_overview, pr_list_open, pr_get_diff"));
        assert!(msg.contains("\"tool_args\""));
        assert!(msg.contains("```json"));
    }

    #[test]
    fn normalize_tool_name_single() {
        let (name, pasted) = normalize_tool_name("pr_list_open");
        assert_eq!(name, "pr_list_open");
        assert!(!pasted);
    }

    #[test]
    fn parse_tool_step_accepts_tool_and_params_aliases() {
        let raw = r#"{"tool":"pr_get_overview","params":{"repo":"o/r","pr_number":19263}}"#;
        let step = parse_chat_agent_step(raw).unwrap();
        assert_eq!(step.action, ChatAgentAction::Tool);
        assert_eq!(step.tool_name.as_deref(), Some("pr_get_overview"));
        assert_eq!(
            step.tool_args
                .as_ref()
                .and_then(|a| a.get("pr_number"))
                .and_then(|v| v.as_u64()),
            Some(19263)
        );
    }

    #[test]
    fn parse_tool_step_merges_top_level_args_when_tool_args_empty() {
        let raw = r#"{
            "action":"tool",
            "tool_name":"pr_get_overview",
            "tool_args":{},
            "args":{"repo":"o/r","pr_number":19263}
        }"#;
        let step = parse_chat_agent_step(raw).unwrap();
        assert_eq!(step.tool_name.as_deref(), Some("pr_get_overview"));
        assert_eq!(
            step.tool_args
                .as_ref()
                .and_then(|a| a.get("repo"))
                .and_then(|v| v.as_str()),
            Some("o/r")
        );
        assert_eq!(
            step.tool_args
                .as_ref()
                .and_then(|a| a.get("pr_number"))
                .and_then(|v| v.as_u64()),
            Some(19263)
        );
    }

    #[test]
    fn parse_tool_step_preserves_tool_args_over_params_alias() {
        let raw = r#"{
            "action":"tool",
            "tool_name":"pr_get_overview",
            "tool_args":{"pr_number":19263},
            "params":{"repo":"o/r","pr_number":1}
        }"#;
        let step = parse_chat_agent_step(raw).unwrap();
        assert_eq!(
            step.tool_args
                .as_ref()
                .and_then(|a| a.get("repo"))
                .and_then(|v| v.as_str()),
            Some("o/r")
        );
        assert_eq!(
            step.tool_args
                .as_ref()
                .and_then(|a| a.get("pr_number"))
                .and_then(|v| v.as_u64()),
            Some(19263)
        );
    }

    #[test]
    fn parse_tool_step_hoists_top_level_tool_params() {
        let raw = r#"{
            "action":"tool",
            "tool_name":"pr_get_overview",
            "repo":"o/r",
            "pr_number":19263
        }"#;
        let step = parse_chat_agent_step(raw).unwrap();
        assert_eq!(
            step.tool_args
                .as_ref()
                .and_then(|a| a.get("repo"))
                .and_then(|v| v.as_str()),
            Some("o/r")
        );
        assert_eq!(
            step.tool_args
                .as_ref()
                .and_then(|a| a.get("pr_number"))
                .and_then(|v| v.as_u64()),
            Some(19263)
        );
    }

    #[test]
    fn parse_tool_call_preserves_nested_args() {
        let raw = r#"{
            "action":"tool",
            "tool_name":"tool_call",
            "tool_args":{
                "name":"pr_get_overview",
                "args":{"repo":"o/r","pr_number":19263}
            }
        }"#;
        let step = parse_chat_agent_step(raw).unwrap();
        assert_eq!(step.tool_name.as_deref(), Some("tool_call"));
        assert_eq!(
            step.tool_args
                .as_ref()
                .and_then(|a| a.get("args"))
                .and_then(|a| a.get("repo"))
                .and_then(|v| v.as_str()),
            Some("o/r")
        );
    }

    #[test]
    fn parse_tool_call_keeps_root_args_as_nested_args() {
        let raw = r#"{
            "action":"tool",
            "tool_name":"tool_call",
            "name":"pr_get_overview",
            "args":{"repo":"o/r","pr_number":19263}
        }"#;
        let step = parse_chat_agent_step(raw).unwrap();
        assert_eq!(
            step.tool_args
                .as_ref()
                .and_then(|a| a.get("args"))
                .and_then(|a| a.get("pr_number"))
                .and_then(|v| v.as_u64()),
            Some(19263)
        );
    }

    #[test]
    fn salvage_compound_hallucinated_tool_name() {
        let raw = r#"{"action":"tool","tool_name":"pr_get_overview_and_changed_files_combined_for_prs_19264_19263"}"#;
        let step = parse_chat_agent_step(raw).unwrap();
        assert_eq!(step.tool_name.as_deref(), Some("pr_get_overview"));
        assert_eq!(
            step.tool_args
                .as_ref()
                .and_then(|a| a.get("pr_number"))
                .and_then(|v| v.as_u64()),
            Some(19264)
        );
    }

    #[test]
    fn parse_tool_step_splits_pr_suffix_in_tool_name() {
        let raw = r#"{"action":"tool","tool_name":"pr_get_overview_19258","tool_args":{"repo":"acme/widget"}}"#;
        let step = parse_chat_agent_step(raw).unwrap();
        assert_eq!(step.action, ChatAgentAction::Tool);
        assert_eq!(step.tool_name.as_deref(), Some("pr_get_overview"));
        assert_eq!(
            step.tool_args
                .as_ref()
                .and_then(|a| a.get("pr_number"))
                .and_then(|v| v.as_u64()),
            Some(19258)
        );
    }

    #[test]
    fn split_pr_suffix_leaves_run_id_tools_alone() {
        assert!(split_pr_suffix_from_tool_name("ci_get_run_summary_12345").is_none());
        assert_eq!(
            split_pr_suffix_from_tool_name("pr_list_changed_files_42").unwrap(),
            ("pr_list_changed_files".into(), 42)
        );
    }

    #[test]
    fn sanitize_chat_keeps_thought_in_prose() {
        let msg = "It appears to be a backport fix for key-auth.";
        assert_eq!(sanitize_chat_message(msg), msg);
        let msg2 = "It appears to be a thought: the PR backports a fix.";
        assert_eq!(sanitize_chat_message(msg2), msg2);
    }

    #[test]
    fn reply_truncated_mid_sentence() {
        let msg = "However, based on the previous list of open PRs, I";
        assert!(reply_message_looks_truncated(msg));
    }

    #[test]
    fn reply_complete_sentence_not_truncated() {
        let msg = "However, based on the previous list of open PRs, PR #19239 has failing CI.";
        assert!(!reply_message_looks_truncated(msg));
    }

    #[test]
    fn chat_response_needs_retry_for_valid_json_truncated_message() {
        let raw =
            r#"{"action":"reply","message":"However, based on the previous list of open PRs, I"}"#;
        assert!(chat_response_needs_retry(raw, Some("stop")));
    }

    #[test]
    fn planning_reply_detected() {
        let msg = "Since I cannot access #19242, I will investigate #19238.";
        assert!(reply_looks_like_planning(msg));
        assert!(reply_premature_for_task(
            msg,
            "tell me why CI fails",
            &["pr_get_overview"],
            None,
        ));
    }

    #[test]
    fn change_question_without_change_tools_is_not_premature_by_user_phrase() {
        assert!(!reply_premature_for_task(
            "文件数量较多，修改了",
            "所以到底修改了什么",
            &["pr_get_overview"],
            Some(19258),
        ));
        assert!(!reply_premature_for_task(
            "foo.rs +10/-2",
            "所以到底修改了什么",
            &["pr_list_changed_files"],
            Some(19258),
        ));
    }

    #[test]
    fn reply_claims_cannot_see_changes_detected() {
        assert!(reply_premature_for_task(
            "由于系统限制，无法直接查看 diff 详情",
            "所以到底修改了什么",
            &["pr_get_overview"],
            Some(19258),
        ));
    }

    #[test]
    fn should_compress_reasoning_threshold() {
        assert!(!should_compress_reasoning(true, "short", 480));
        assert!(should_compress_reasoning(true, &"x".repeat(500), 480));
        assert!(!should_compress_reasoning(false, &"x".repeat(500), 480));
    }

    #[test]
    fn streaming_message_only_for_reply_action() {
        let tool_buf = r#"{"action":"tool","tool_name":"pr_list_open","message":"hi"#;
        assert!(extract_streaming_chat_message(tool_buf).is_none());
        assert_eq!(
            extract_streaming_tool_pending(tool_buf).as_deref(),
            Some("pr_list_open")
        );
        let reply_buf = r#"{"action":"reply","message":"Hello"#;
        assert_eq!(
            extract_streaming_chat_message(reply_buf).as_deref(),
            Some("Hello")
        );
        assert!(extract_streaming_tool_pending(reply_buf).is_none());
    }

    #[test]
    fn tool_result_transcript_is_not_streamed_as_reply() {
        let buf =
            r#"{"action":"reply","message":"tool_result(pr_list_changed_files, pr_number=1):"#;
        assert!(extract_streaming_chat_message(buf).is_none());
    }

    #[test]
    fn tool_message_before_action_is_not_streamed_as_reply() {
        let buf = r#"{"message":"AI Tool: pr_list_changed_files","action":"tool","tool_name":"pr_list_changed_files"#;
        assert!(extract_streaming_chat_message(buf).is_none());
        assert_eq!(
            extract_streaming_tool_pending(buf).as_deref(),
            Some("pr_list_changed_files")
        );
    }

    #[test]
    fn reply_substring_inside_tool_message_does_not_false_positive() {
        let buf = r#"{"action":"tool","message":"do not use \"action\":\"reply\" here","tool_name":"pr_get_overview"#;
        assert!(extract_streaming_chat_message(buf).is_none());
    }

    #[test]
    fn tool_narration_in_reply_action_is_not_streamed() {
        let buf = r#"{"action":"reply","message":"AI Tool: pr_list_open\nArgs: {}"#;
        assert!(extract_streaming_chat_message(buf).is_none());
    }

    #[test]
    fn select_best_prefers_thinking_json_when_content_lacks_repo() {
        let content =
            r#"{"action":"tool","tool_name":"pr_get_overview","tool_args":{"pr_number":18286}}"#;
        let thinking = "plan...\n{\"action\":\"tool\",\"tool_name\":\"pr_get_overview\",\"tool_args\":{\"repo\":\"acme/widget\",\"pr_number\":18286}}";
        let picked = select_best_chat_step_text(content, thinking);
        assert!(picked.contains("acme/widget"));
        let step = parse_chat_agent_step(&picked).unwrap();
        assert_eq!(
            step.tool_args
                .as_ref()
                .and_then(|a| a.get("repo"))
                .and_then(|v| v.as_str()),
            Some("acme/widget")
        );
    }

    #[test]
    fn flatten_nested_params_into_top_level_tool_args() {
        let mut args = serde_json::json!({"params": {"repo": "o/r", "pr_number": 1}});
        flatten_tool_args(&mut args);
        assert_eq!(args["repo"], serde_json::json!("o/r"));
        assert_eq!(args["pr_number"], serde_json::json!(1));
    }

    #[test]
    fn flatten_stringified_tool_args_object() {
        let mut args = serde_json::json!(r#"{"repo":"o/r","pr_number":2}"#);
        flatten_tool_args(&mut args);
        assert_eq!(args["repo"], serde_json::json!("o/r"));
    }
}
