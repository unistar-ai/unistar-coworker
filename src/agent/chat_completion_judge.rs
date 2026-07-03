//! LLM gate for whether a bare assistant reply should end the user turn.

use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::{CoworkerError, Result};
use crate::llm::{LlmClient, LlmTurnMessage};

pub const COMPLETION_JUDGE_PROMPT: &str = include_str!("../../prompts/chat-completion-judge.md");
const COMPLETION_JUDGE_MAX_TOKENS: u32 = 384;
const JUDGE_TRANSCRIPT_MAX_CHARS: usize = 14_000;
const JUDGE_LINE_MAX_CHARS: usize = 1_200;

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

pub fn parse_completion_judge_response(content: &str) -> Result<TurnCompletionVerdict> {
    let trimmed = content.trim();
    if let Ok(verdict) = serde_json::from_str::<TurnCompletionVerdict>(trimmed) {
        return Ok(verdict);
    }
    let unfenced = strip_json_fence(trimmed);
    if unfenced != trimmed {
        if let Ok(verdict) = serde_json::from_str::<TurnCompletionVerdict>(&unfenced) {
            return Ok(verdict);
        }
    }
    if let Some(start) = trimmed.find('{') {
        if let Some(end) = trimmed.rfind('}') {
            if end > start {
                let slice = &trimmed[start..=end];
                if let Ok(verdict) = serde_json::from_str::<TurnCompletionVerdict>(slice) {
                    return Ok(verdict);
                }
            }
        }
    }
    Err(CoworkerError::Workflow(format!(
        "completion judge returned invalid JSON: {}",
        trimmed.chars().take(240).collect::<String>()
    )))
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
        "## Conversation context (recent)".to_string(),
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

pub async fn judge_turn_completion(
    llm: &LlmClient,
    user_task: &str,
    llm_messages: &[LlmTurnMessage],
    tools_this_turn: &[ToolUseSummary],
    proposed_reply: &str,
) -> Result<TurnCompletionVerdict> {
    let payload =
        format_completion_judge_payload(user_task, llm_messages, tools_this_turn, proposed_reply);
    let raw = llm
        .judge_chat_turn_complete_json(
            COMPLETION_JUDGE_PROMPT,
            &payload,
            &completion_judge_response_schema(),
            COMPLETION_JUDGE_MAX_TOKENS,
        )
        .await?;
    parse_completion_judge_response(&raw)
}

fn format_tools_this_turn(tools: &[ToolUseSummary]) -> String {
    if tools.is_empty() {
        return "(none)".to_string();
    }
    tools
        .iter()
        .map(|tc| format!("- {}: {}", tc.name, tc.output_preview))
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
        return format!("[tool {name}] {}", truncate_line(&msg.content));
    }
    if msg.role == "user" && msg.content.contains("[Harness]") {
        let first = msg.content.lines().next().unwrap_or("[harness]");
        return format!("[{role}] {}", truncate_line(first));
    }
    format!("[{role}] {}", truncate_line(&msg.content))
}

fn truncate_line(text: &str) -> String {
    let t = text.trim();
    if t.chars().count() <= JUDGE_LINE_MAX_CHARS {
        return t.to_string();
    }
    format!(
        "{}…",
        t.chars().take(JUDGE_LINE_MAX_CHARS).collect::<String>()
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
