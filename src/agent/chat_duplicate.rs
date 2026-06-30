//! Guards against duplicate tool calls within a chat turn.
//!
//! Despite the name, this module is about *tool-call* duplication detection
//! and harness nudges — not code-duplication.

use std::collections::HashSet;

use serde_json::Value;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::agent::chat_loop::{
    append_message, canonical_tool_args, emit_progress, format_tool_args_short, is_mutating_tool,
    push_native_assistant_tool_calls, ChatProgress, PreparedToolCall, ToolCallSummary,
    ToolExecRecord, ToolRoundState,
};
use crate::agent::context::{format_tool_context_message, harness_nudge_base};
use crate::agent::tool_catalog;
use crate::app::AppEvent;
use crate::engine::SkillRegistry;
use crate::error::Result;
use crate::llm::chat::ChatAgentStep;
use crate::llm::LlmTurnMessage;
use crate::store::{ChatRole, Store};

/// Max harness-only LLM retries per user turn (missing args, malformed JSON, etc.).
pub(crate) const MAX_HARNESS_CORRECTIONS: u32 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DuplicateToolBlock {
    AlreadySucceeded,
    FailedTooMany,
}

pub(crate) fn duplicate_tool_block_reason(
    record: Option<&ToolExecRecord>,
) -> Option<DuplicateToolBlock> {
    let record = record?;
    if record.succeeded {
        return Some(DuplicateToolBlock::AlreadySucceeded);
    }
    if record.fail_count >= 2 {
        return Some(DuplicateToolBlock::FailedTooMany);
    }
    None
}

pub(crate) fn forced_reply_after_duplicate_tools_nudge(
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

pub(crate) fn duplicate_tool_nudge(tool_name: &str, block: DuplicateToolBlock) -> String {
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

pub(crate) fn maybe_push_tool_failure_harness_nudge(
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
        .filter(|field| !tool_catalog::ToolCatalog::tool_arg_field_satisfied(effective_args, field))
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

pub(crate) fn push_harness_nudge(messages: &mut Vec<LlmTurnMessage>, content: String) -> String {
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

pub(crate) fn remove_satisfied_missing_arg_nudges(
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

pub(crate) fn prune_stale_missing_arg_nudges(messages: &mut Vec<LlmTurnMessage>) {
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

async fn persist_harness_nudge(
    store: &dyn Store,
    session_id: &Uuid,
    llm_messages: &mut Vec<LlmTurnMessage>,
    nudge: &str,
) -> Result<()> {
    let body = push_harness_nudge(llm_messages, nudge.to_string());
    append_message(
        store,
        session_id,
        ChatRole::Harness,
        &body,
        None,
        None,
        None,
    )
    .await
}

pub(crate) async fn harness_retry_or_stop(
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
pub(crate) enum CachedToolOutput {
    /// Already formatted `tool_result(...)` transcript from session context.
    Transcript(String),
    /// Raw tool body to wrap in a new transcript.
    Body(String),
}

pub(crate) async fn fulfill_duplicate_readonly_tool(
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
            None,
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

pub(crate) fn find_prior_tool_result_body(
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

pub(crate) fn tool_transcript_matches_args(content: &str, args: &Value, want_fp: &str) -> bool {
    if let Some(args_line) = content
        .lines()
        .find(|line| line.trim_start().starts_with("args:"))
    {
        let json_part = args_line
            .trim_start()
            .strip_prefix("args:")
            .unwrap_or("")
            .trim();
        if let Ok(parsed) = serde_json::from_str::<Value>(json_part) {
            if canonical_tool_args(&parsed) == want_fp {
                return true;
            }
        }
    }
    content.contains(&args.to_string()) || canonical_tool_args(args) == want_fp
}

pub(crate) async fn maybe_block_duplicate_tool_call(
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::context::format_tool_context_message;
    use serde_json::json;

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
        let content = format_tool_context_message("skill_load", &args, true, "### pr-review\nbody");
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
        use crate::agent::chat_loop::tool_call_fingerprint;
        use std::collections::HashMap;
        let fp = tool_call_fingerprint("pr_list_open", &json!({"repo": "acme/widget"}));
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
}
