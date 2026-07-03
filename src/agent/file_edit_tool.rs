//! LLM-reviewed `edit_file` / `write_file` (no human approval queue).

use std::path::Path;

use serde_json::Value;

use crate::agent::bash_tool::{
    bash_review_response_schema, parse_bash_review_response_for_tool, BashCommandReview,
    REVIEW_JSON_RETRY_SUFFIX,
};
use crate::agent::context::truncate_chars;
use crate::agent::file_tools::{self, EDIT_FILE, WRITE_FILE};
use crate::agent::harness_errors::{file_edit_preflight_envelope, file_edit_validation_envelope};
use crate::agent::review_gate::ReviewGateOutcome;
use crate::error::{CoworkerError, Result};
use crate::llm::LlmClient;

const FILE_EDIT_REVIEW_PROMPT: &str = include_str!("../../prompts/file-edit-review.md");
const FILE_EDIT_REVIEW_MAX_TOKENS: u32 = 1024;
const FILE_EDIT_REVIEW_SNIPPET_CHARS: usize = 6_000;

pub async fn execute_mutating_file_tool_with_review(
    workspace: &Path,
    llm: &LlmClient,
    name: &str,
    args: &Value,
) -> Result<ReviewGateOutcome> {
    let payload = build_review_payload(name, args)?;
    if let Some(env) = file_edit_preflight_envelope(name, args) {
        return Err(CoworkerError::Workflow(env.format_tool_error_body()));
    }

    let review = review_file_edit(llm, name, &payload).await?;
    if !review.is_approved() {
        return Ok(ReviewGateOutcome::LlmRejected(review));
    }

    let mut out = file_tools::execute_mutating_file_tool(workspace, name, args)?;
    if !out.starts_with("review:") {
        out = format!("review: APPROVE ({})\n{out}", review.reason_code);
    }
    Ok(ReviewGateOutcome::Executed(out))
}

fn build_review_payload(name: &str, args: &Value) -> Result<String> {
    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            CoworkerError::Workflow(
                file_edit_validation_envelope(name, "missing path", args).format_tool_error_body(),
            )
        })?;

    let mut body = format!("tool: {name}\npath: {path}\n");
    match name {
        EDIT_FILE => {
            let old_string = args
                .get("old_string")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    CoworkerError::Workflow(
                        file_edit_validation_envelope(name, "edit_file needs old_string", args)
                            .format_tool_error_body(),
                    )
                })?;
            let new_string = args
                .get("new_string")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    CoworkerError::Workflow(
                        file_edit_validation_envelope(name, "edit_file needs new_string", args)
                            .format_tool_error_body(),
                    )
                })?;
            let replace_all = args
                .get("replace_all")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            body.push_str(&format!("replace_all: {replace_all}\n"));
            body.push_str("--- old_string ---\n");
            body.push_str(&truncate_chars(old_string, FILE_EDIT_REVIEW_SNIPPET_CHARS));
            body.push_str("\n--- new_string ---\n");
            body.push_str(&truncate_chars(new_string, FILE_EDIT_REVIEW_SNIPPET_CHARS));
        }
        WRITE_FILE => {
            let content = args
                .get("content")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    CoworkerError::Workflow(
                        file_edit_validation_envelope(name, "write_file needs content", args)
                            .format_tool_error_body(),
                    )
                })?;
            let create_only = args
                .get("create_only")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            body.push_str(&format!("create_only: {create_only}\n"));
            body.push_str("--- content ---\n");
            body.push_str(&truncate_chars(content, FILE_EDIT_REVIEW_SNIPPET_CHARS));
        }
        other => {
            return Err(CoworkerError::Workflow(
                file_edit_validation_envelope(other, "unknown file edit tool", args)
                    .format_tool_error_body(),
            ));
        }
    }
    Ok(body)
}

async fn review_file_edit(
    llm: &LlmClient,
    tool_name: &str,
    payload: &str,
) -> Result<BashCommandReview> {
    let schema = bash_review_response_schema();
    let raw = llm
        .review_file_edit_json(
            FILE_EDIT_REVIEW_PROMPT,
            payload,
            &schema,
            FILE_EDIT_REVIEW_MAX_TOKENS,
        )
        .await?;
    if let Ok(review) = parse_bash_review_response_for_tool(&raw, tool_name) {
        return Ok(review);
    }
    tracing::warn!("{tool_name} review JSON parse failed, retrying with JSON-only nudge");
    let retry_prompt = format!("{FILE_EDIT_REVIEW_PROMPT}{REVIEW_JSON_RETRY_SUFFIX}");
    let raw = llm
        .review_file_edit_json(&retry_prompt, payload, &schema, FILE_EDIT_REVIEW_MAX_TOKENS)
        .await?;
    parse_bash_review_response_for_tool(&raw, tool_name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::bash_tool::BashCriticalIssue;
    use crate::agent::harness_errors::file_edit_safety_reject_envelope;
    use serde_json::json;

    #[test]
    fn build_review_payload_edit() {
        let payload = build_review_payload(
            EDIT_FILE,
            &json!({
                "path": "src/a.rs",
                "old_string": "fn old() {}",
                "new_string": "fn new() {}"
            }),
        )
        .unwrap();
        assert!(payload.contains("tool: edit_file"));
        assert!(payload.contains("fn old()"));
    }

    #[test]
    fn preflight_blocks_env_path() {
        assert!(file_edit_preflight_envelope(
            WRITE_FILE,
            &json!({ "path": ".env", "content": "X=1" })
        )
        .is_some());
    }

    #[test]
    fn safety_reject_envelope_has_harness_marker() {
        let review = BashCommandReview {
            verdict: "REJECT".into(),
            reason_code: "RISK_FOUND".into(),
            critical_issues: vec![BashCriticalIssue {
                line_number: 1,
                code_snippet: "old_string too short".into(),
                risk_type: "MISSING_ERROR_HANDLING".into(),
                description: "ambiguous edit".into(),
            }],
            suggestions: vec!["read_file first".into()],
        };
        let env = file_edit_safety_reject_envelope(
            EDIT_FILE,
            &json!({ "path": "a.rs", "old_string": "x", "new_string": "y" }),
            &review,
        );
        assert!(env.format_harness_nudge().contains("[Harness]"));
    }
}
