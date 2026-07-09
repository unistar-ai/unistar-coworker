//! Native `ask_user` tool — pause the turn and wait for a human answer.

use serde_json::Value;
use uuid::Uuid;

use crate::error::{CoworkerError, Result};

pub const ASK_USER_TOOL: &str = "ask_user";

#[derive(Debug, Clone)]
pub struct AskUserRequest {
    pub question: String,
    pub options: Vec<String>,
    pub context: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AskUserPause {
    pub question_id: Uuid,
    pub request: AskUserRequest,
    pub tool_call_id: String,
    pub tool_args: Value,
}

pub fn is_ask_user_tool(name: &str) -> bool {
    name == ASK_USER_TOOL
}

pub fn parse_ask_user_args(args: &Value) -> Result<AskUserRequest> {
    let question = args
        .get("question")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            CoworkerError::Workflow(
                "ask_user requires non-empty `question` (what to ask the user)".into(),
            )
        })?
        .to_string();

    let options = args
        .get("options")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.trim().to_string()))
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let context = args
        .get("context")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    Ok(AskUserRequest {
        question,
        options,
        context,
    })
}

pub fn format_user_answer_body(answer: &str) -> String {
    format!("User answered:\n{}", answer.trim())
}

/// Human-readable body stored while the turn is paused on `ask_user`.
pub fn format_pending_question_body(request: &AskUserRequest) -> String {
    let mut body = format!("Awaiting user answer.\n\nQuestion: {}", request.question);
    if let Some(ctx) = &request.context {
        body.push_str("\n\nContext: ");
        body.push_str(ctx);
    }
    if !request.options.is_empty() {
        body.push_str("\n\nOptions:");
        for (i, opt) in request.options.iter().enumerate() {
            body.push_str(&format!("\n  {}. {opt}", i + 1));
        }
    }
    body
}

/// Parse `question_id` from a `tool_user_question_pending(...)` transcript header.
pub fn question_id_from_pending_transcript(content: &str) -> Option<Uuid> {
    let trimmed = content.trim_start();
    let rest = trimmed.strip_prefix("tool_user_question_pending(")?;
    let header = rest.split("):").next()?;
    for part in header.split(',') {
        let part = part.trim();
        if let Some(id) = part.strip_prefix("question_id=") {
            return Uuid::parse_str(id.trim()).ok();
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_requires_question() {
        assert!(parse_ask_user_args(&json!({})).is_err());
        assert!(parse_ask_user_args(&json!({"question": "  "})).is_err());
    }

    #[test]
    fn parse_question_and_options() {
        let req = parse_ask_user_args(&json!({
            "question": "Which repo?",
            "options": ["acme/widget", "acme/api", ""],
            "context": "Need repo for pr_list_open"
        }))
        .unwrap();
        assert_eq!(req.question, "Which repo?");
        assert_eq!(req.options, vec!["acme/widget", "acme/api"]);
        assert_eq!(req.context.as_deref(), Some("Need repo for pr_list_open"));
    }

    #[test]
    fn extracts_question_id_from_pending_transcript() {
        let id = Uuid::new_v4();
        let text = format!(
            "tool_user_question_pending(ask_user, question_id={id}):\nargs: {{}}\n\nAwaiting"
        );
        assert_eq!(super::question_id_from_pending_transcript(&text), Some(id));
        assert!(super::question_id_from_pending_transcript("tool_result(ask_user)").is_none());
    }
}
