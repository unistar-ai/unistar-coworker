//! LLM gate for whether a bare assistant reply should end the user turn.

use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::Result;
use crate::llm::{LlmClient, LlmTurnMessage};

pub const COMPLETION_JUDGE_PROMPT: &str =
    include_str!("../../../../prompts/chat-completion-judge.md");
const COMPLETION_JUDGE_MAX_TOKENS: u32 = 256;
const JUDGE_TRANSCRIPT_MAX_CHARS: usize = 6_000;
const JUDGE_LINE_MAX_CHARS: usize = 400;
const JUDGE_TOOL_PREVIEW_MAX_CHARS: usize = 120;

#[derive(Debug, Clone)]
pub struct ToolUseSummary {
    pub name: String,
    pub output_preview: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct TurnCompletionVerdict {
    pub complete: bool,
    #[serde(default)]
    pub reason: String,
}

pub fn completion_judge_response_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "complete": { "type": "boolean" },
            "reason": { "type": "string" }
        },
        "required": ["complete"],
        "additionalProperties": false
    })
}

/// Parse judge output; returns `None` when the model did not emit a verdict object.
pub fn parse_completion_judge_response(content: &str) -> Option<TurnCompletionVerdict> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(verdict) = serde_json::from_str::<TurnCompletionVerdict>(trimmed) {
        return Some(verdict);
    }
    let unfenced = strip_json_fence(trimmed);
    if unfenced != trimmed {
        if let Ok(verdict) = serde_json::from_str::<TurnCompletionVerdict>(&unfenced) {
            return Some(verdict);
        }
    }
    for slice in extract_json_object_slices(trimmed) {
        if let Ok(verdict) = serde_json::from_str::<TurnCompletionVerdict>(&slice) {
            if looks_like_verdict_object(&slice) {
                return Some(verdict);
            }
        }
    }
    parse_completion_verdict_heuristic(trimmed)
}

fn looks_like_verdict_object(slice: &str) -> bool {
    slice.contains("\"complete\"")
        || slice.contains("'complete'")
        || slice.contains("complete:")
}

fn parse_completion_verdict_heuristic(text: &str) -> Option<TurnCompletionVerdict> {
    let lower = text.to_ascii_lowercase();
    if let Some(idx) = lower.find("\"complete\"").or_else(|| lower.find("complete")) {
        let tail = &lower[idx..];
        if tail.contains("true") && !tail.contains("false") {
            return Some(TurnCompletionVerdict {
                complete: true,
                reason: String::new(),
            });
        }
        if tail.contains("false") {
            let reason = extract_reason_heuristic(text).unwrap_or_default();
            return Some(TurnCompletionVerdict { complete: false, reason });
        }
    }
    None
}

fn extract_reason_heuristic(text: &str) -> Option<String> {
    let re = regex::Regex::new(r#""reason"\s*:\s*"([^"]*)""#).ok()?;
    re.captures(text)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
        .filter(|s| !s.is_empty())
}

fn extract_json_object_slices(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start = None;
    for (i, ch) in text.char_indices() {
        match ch {
            '{' => {
                if depth == 0 {
                    start = Some(i);
                }
                depth += 1;
            }
            '}' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    if let Some(s) = start {
                        out.push(text[s..=i].to_string());
                        start = None;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

pub fn format_completion_judge_payload(
    user_task: &str,
    llm_messages: &[LlmTurnMessage],
    tools_this_turn: &[ToolUseSummary],
    proposed_reply: &str,
) -> String {
    [
        format!("## User task\n{}", user_task.trim()),
        format!(
            "## Tools run this turn ({})\n{}",
            tools_this_turn.len(),
            format_tools_this_turn(tools_this_turn)
        ),
        "## Proposed assistant reply (no tool calls)".to_string(),
        proposed_reply.trim().to_string(),
        "## Conversation context (recent, abbreviated)".to_string(),
        format_recent_transcript(llm_messages),
    ]
    .join("\n\n")
}

pub fn completion_rejected_nudge(reason: &str) -> String {
    let detail = reason.trim();
    if detail.is_empty() {
        return "Completion check: the task is not finished yet. \
Continue with tool calls or reply with a complete synthesis when done."
            .to_string();
    }
    format!(
        "Completion check: the task is not finished yet.\n\n{detail}\n\n\
Continue with tool calls or reply with a complete answer when done."
    )
}

/// Fail-open when the judge model returns garbage or nothing — better than looping forever.
fn completion_judge_fail_open(reason: &str) -> TurnCompletionVerdict {
    tracing::warn!("completion judge fail-open: {reason}");
    TurnCompletionVerdict {
        complete: true,
        reason: String::new(),
    }
}

pub async fn judge_turn_completion(
    llm: &LlmClient,
    user_task: &str,
    llm_messages: &[LlmTurnMessage],
    tools_this_turn: &[ToolUseSummary],
    proposed_reply: &str,
) -> Result<TurnCompletionVerdict> {
    let payload =
        format_completion_judge_payload(user_task, llm_messages, tools_this_turn, proposed_reply);
    let raw = match llm
        .judge_chat_turn_complete_json(
            COMPLETION_JUDGE_PROMPT,
            &payload,
            &completion_judge_response_schema(),
            COMPLETION_JUDGE_MAX_TOKENS,
        )
        .await
    {
        Ok(s) if !s.trim().is_empty() => s,
        Ok(_) => {
            return Ok(completion_judge_fail_open("empty judge response"));
        }
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("empty message content") || msg.contains("llm offline") {
                return Ok(completion_judge_fail_open(&msg));
            }
            return Err(e);
        }
    };
    match parse_completion_judge_response(&raw) {
        Some(verdict) => Ok(verdict),
        None => Ok(completion_judge_fail_open(&format!(
            "unparseable judge JSON: {}",
            raw.chars().take(120).collect::<String>()
        ))),
    }
}

fn format_tools_this_turn(tools: &[ToolUseSummary]) -> String {
    if tools.is_empty() {
        return "(none)".to_string();
    }
    tools
        .iter()
        .map(|tc| {
            format!(
                "- {}: {}",
                tc.name,
                sanitize_judge_snippet(&tc.output_preview, JUDGE_TOOL_PREVIEW_MAX_CHARS)
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_recent_transcript(messages: &[LlmTurnMessage]) -> String {
    let mut lines = Vec::new();
    let mut used = 0usize;
    for msg in messages.iter().rev() {
        if msg.role == "system" {
            continue;
        }
        let line = format_transcript_line(msg);
        let line_len = line.chars().count();
        if used + line_len > JUDGE_TRANSCRIPT_MAX_CHARS {
            break;
        }
        used += line_len;
        lines.push(line);
    }
    lines.reverse();
    if lines.is_empty() {
        "(empty)".to_string()
    } else {
        lines.join("\n\n")
    }
}

fn format_transcript_line(msg: &LlmTurnMessage) -> String {
    let role = msg.role;
    if let Some(calls) = &msg.tool_calls {
        let names: Vec<_> = calls.iter().map(|c| c.name.as_str()).collect();
        return format!("[{role}] tool_calls: {}", names.join(", "));
    }
    if msg.role == "tool" {
        let name = msg.tool_name.as_deref().unwrap_or("tool");
        let body = sanitize_judge_snippet(&msg.content, JUDGE_LINE_MAX_CHARS);
        return format!("[tool {name}] {body}");
    }
    if msg.role == "user" && msg.content.contains("[Harness]") {
        let first = msg.content.lines().next().unwrap_or("[harness]");
        return format!("[{role}] {}", sanitize_judge_snippet(first, JUDGE_LINE_MAX_CHARS));
    }
    format!(
        "[{role}] {}",
        sanitize_judge_snippet(&msg.content, JUDGE_LINE_MAX_CHARS)
    )
}

/// Keep judge context small and avoid echoing huge JSON blobs back into the model.
fn sanitize_judge_snippet(text: &str, max_chars: usize) -> String {
    let t = text.trim();
    if t.is_empty() {
        return String::new();
    }
    let line_count = t.lines().count();
    let looks_like_commit_dump = t.contains("\"sha\"")
        || t.contains("diff --git")
        || (t.contains('{') && t.contains("\"author\"") && line_count > 3);
    if looks_like_commit_dump {
        return format!("({line_count} lines of tool output omitted for judge)");
    }
    if t.chars().count() <= max_chars {
        return t.to_string();
    }
    format!(
        "{}…",
        t.chars().take(max_chars).collect::<String>()
    )
}

fn strip_json_fence(text: &str) -> String {
    let trimmed = text.trim();
    if let Some(rest) = trimmed.strip_prefix("```json") {
        return rest.trim_end_matches("```").trim().to_string();
    }
    if let Some(rest) = trimmed.strip_prefix("```") {
        return rest.trim_end_matches("```").trim().to_string();
    }
    trimmed.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_completion_verdict_json() {
        let v = parse_completion_judge_response(
            r#"{"complete": false, "reason": "still investigating"}"#,
        )
        .unwrap();
        assert!(!v.complete);
        assert_eq!(v.reason, "still investigating");
    }

    #[test]
    fn parse_completion_verdict_from_fence() {
        let v = parse_completion_judge_response("```json\n{\"complete\": true}\n```").unwrap();
        assert!(v.complete);
    }

    #[test]
    fn parse_rejects_commit_echo_without_complete_field() {
        assert!(parse_completion_judge_response(
            r#"{"author":"windmgc","commit":"chore(ci): add test","sha":"abc123"}"#
        )
        .is_none());
    }

    #[test]
    fn parse_heuristic_complete_true() {
        let v = parse_completion_judge_response(r#"Sure. {"complete": true}"#).unwrap();
        assert!(v.complete);
    }

    #[test]
    fn sanitize_strips_commit_json_dump() {
        let raw = r#"{"sha":"abc","author":"x","commit":"msg"}
{"sha":"def","author":"y","commit":"msg2"}"#;
        let s = sanitize_judge_snippet(raw, 400);
        assert!(s.contains("omitted for judge"));
    }

    #[test]
    fn format_payload_includes_task_and_reply() {
        let payload = format_completion_judge_payload(
            "why CI fails",
            &[LlmTurnMessage::new("user", "check PR 42")],
            &[ToolUseSummary {
                name: "bash_run".into(),
                output_preview: "ok".into(),
            }],
            "Let me fetch logs next.",
        );
        assert!(payload.contains("why CI fails"));
        assert!(payload.contains("bash_run"));
        assert!(payload.contains("Let me fetch logs"));
    }
}
