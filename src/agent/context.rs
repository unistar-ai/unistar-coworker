//! Token-aware chat context packing for 64K (and other) context windows.

use crate::agent::budget::TokenBudget;
use crate::agent::parse::{parse_issue_line, parse_pr_line};
use crate::error::Result;
use crate::llm::{LlmClient, LlmTurnMessage};
use crate::store::{ChatMessage, ChatRole};
use serde_json::Value;

/// Rough token estimate (~4 chars per token for Latin/mixed text).
pub fn estimate_tokens(text: &str) -> u32 {
    (text.len() as u32 / 4).max(1)
}

pub fn estimate_messages_tokens(messages: &[LlmTurnMessage]) -> u32 {
    messages.iter().map(estimate_message_tokens).sum()
}

pub fn estimate_message_tokens(msg: &LlmTurnMessage) -> u32 {
    let mut n = estimate_tokens(&msg.content).saturating_add(4);
    if let Some(calls) = &msg.tool_calls {
        for tc in calls {
            n = n.saturating_add(estimate_tokens(&tc.name).saturating_add(8));
            if let Ok(s) = serde_json::to_string(&tc.arguments) {
                n = n.saturating_add(estimate_tokens(&s));
            }
        }
    }
    if let Some(name) = &msg.tool_name {
        n = n.saturating_add(estimate_tokens(name));
    }
    n
}

/// Body text for the LLM context panel (includes native `tool_calls` when prose is empty).
pub fn format_llm_message_for_context_panel(msg: &LlmTurnMessage) -> String {
    let prose = msg.content.trim();
    let mut parts = Vec::new();
    if !prose.is_empty() {
        parts.push(msg.content.clone());
    }
    if let Some(calls) = &msg.tool_calls {
        for tc in calls {
            let args = serde_json::to_string_pretty(&tc.arguments)
                .unwrap_or_else(|_| tc.arguments.to_string());
            parts.push(format!("tool_call: {}\nargs: {args}", tc.name));
        }
    }
    if parts.is_empty() {
        msg.content.clone()
    } else {
        parts.join("\n\n")
    }
}

/// Truncate at a Unicode scalar boundary (safe for CJK and emoji).
pub fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        text.to_string()
    } else {
        format!("{}…", text.chars().take(max_chars).collect::<String>())
    }
}

/// Per-tool byte cap before a result enters LLM context (current turn).
pub fn tool_result_char_cap(tool_name: &str) -> usize {
    match tool_name {
        "pr_list_open" | "pr_list_merged" | "pr_list_waiting_review" | "issue_list_open" => 2_800,
        "ci_get_failed_logs" => 4_800,
        "pr_list_changed_files" => 3_200,
        "pr_get_diff" => 4_000,
        "pr_get_overview" | "ci_analyze_pr_failures" | "ci_get_run_summary" => 3_500,
        "store_get_latest_digest" => 2_000,
        _ => 6_000,
    }
}

pub fn cap_tool_result(tool_name: &str, text: &str) -> String {
    let cap = tool_result_char_cap(tool_name);
    if text.chars().count() <= cap {
        return text.to_string();
    }
    format!(
        "{}…\n[truncated {} chars — use a narrower tool or follow-up for full output]",
        truncate_chars(text, cap),
        text.chars().count().saturating_sub(cap)
    )
}

fn structure_pr_list_output(text: &str) -> String {
    let mut lines = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with("open PR")
            || trimmed.starts_with("stale open PR")
            || trimmed.starts_with("merged PR")
            || trimmed.contains(" waiting for review in ")
            || trimmed.starts_with('(')
        {
            lines.push(trimmed.to_string());
            continue;
        }
        if trimmed.starts_with('#') && trimmed.contains(" CI:") && !trimmed.contains('@') {
            lines.push(trimmed.to_string());
            continue;
        }
        if let Some(pr) = parse_pr_line(trimmed) {
            lines.push(format!("#{} CI:{}", pr.number, pr.ci));
        }
    }
    if lines.is_empty() {
        text.to_string()
    } else {
        lines.join("\n")
    }
}

fn structure_issue_list_output(text: &str) -> String {
    let mut lines = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with("open issue") || trimmed.starts_with('(') {
            lines.push(trimmed.to_string());
            continue;
        }
        if let Some(issue) = parse_issue_line(trimmed) {
            lines.push(format!("#{} labels:{}", issue.number, issue.labels));
        }
    }
    if lines.is_empty() {
        text.to_string()
    } else {
        lines.join("\n")
    }
}

/// Compact verbose list tool output before it enters LLM context.
pub fn structure_tool_result(tool_name: &str, text: &str) -> String {
    match tool_name {
        "pr_list_open" | "pr_list_merged" | "pr_list_waiting_review" => {
            structure_pr_list_output(text)
        }
        "issue_list_open" => structure_issue_list_output(text),
        "pr_list_changed_files" => structure_pr_changed_files_output(text),
        "pr_get_diff" => structure_pr_get_diff_output(text),
        _ => text.to_string(),
    }
}

fn structure_pr_changed_files_output(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return text.to_string();
    }
    // Already compact from MCP; preserve header + totals line.
    trimmed.to_string()
}

fn split_unified_diff(diff: &str) -> Vec<(String, String)> {
    let mut chunks = Vec::new();
    let mut current_path = String::new();
    let mut current_body = String::new();
    for line in diff.lines() {
        if line.starts_with("diff --git ") {
            if !current_path.is_empty() {
                chunks.push((current_path.clone(), current_body.clone()));
            }
            current_path = line
                .strip_prefix("diff --git ")
                .and_then(|s| s.split_whitespace().nth(1))
                .unwrap_or(line)
                .trim_start_matches("b/")
                .to_string();
            current_body.clear();
        } else {
            current_body.push_str(line);
            current_body.push('\n');
        }
    }
    if !current_path.is_empty() {
        chunks.push((current_path, current_body));
    }
    chunks
}

/// Per-file diff excerpts so large PRs fit context without losing file names.
pub fn structure_pr_get_diff_output(text: &str) -> String {
    const MAX_FILES: usize = 12;
    const PER_FILE_CHARS: usize = 720;

    let body = text
        .strip_prefix("Diff for ")
        .and_then(|s| s.split_once(":\n\n").map(|(_, body)| body))
        .unwrap_or(text);

    let files = split_unified_diff(body);
    if files.is_empty() {
        return cap_tool_result("pr_get_diff", text);
    }

    let mut out = String::new();
    if let Some(header) = text.lines().next() {
        out.push_str(header);
        out.push_str("\n\nPer-file excerpts (harness-compressed):\n");
    } else {
        out.push_str("Per-file diff excerpts:\n");
    }

    for (path, chunk) in files.iter().take(MAX_FILES) {
        let excerpt: String = chunk.chars().take(PER_FILE_CHARS).collect();
        let truncated = chunk.len() > PER_FILE_CHARS;
        out.push_str(&format!("\n### {path} ({} lines", chunk.lines().count()));
        if truncated {
            out.push_str(", excerpt");
        }
        out.push_str(")\n");
        out.push_str(excerpt.trim_end());
        if truncated {
            out.push_str("\n… [file diff truncated]");
        }
        out.push('\n');
    }
    if files.len() > MAX_FILES {
        out.push_str(&format!(
            "\n… and {} more file(s) — call pr_list_changed_files for the full path list",
            files.len() - MAX_FILES
        ));
    }
    if text.contains("[diff truncated at max_bytes]") {
        out.push_str("\n[upstream diff truncated at max_bytes — summarize listed files only]");
    }
    cap_tool_result("pr_get_diff", &out)
}

/// Structure and cap a tool result for LLM context.
pub fn prepare_tool_result_for_context(tool_name: &str, text: &str) -> String {
    cap_tool_result(tool_name, &structure_tool_result(tool_name, text))
}

/// Full tool transcript for LLM context: header, complete args JSON, and uncapped body.
pub fn format_tool_context_message(
    tool_name: &str,
    tool_args: &Value,
    ok: bool,
    body: &str,
) -> String {
    let header = if ok {
        if let Some(n) = pr_number_from_tool_args(tool_args) {
            format!("tool_result({tool_name}, pr_number={n})")
        } else {
            format!("tool_result({tool_name})")
        }
    } else {
        format!("tool_error({tool_name})")
    };
    format_tool_transcript_with_header(&header, tool_args, body)
}

/// Mutating tool queued for human approval — not an execution error.
pub fn format_tool_approval_pending_message(
    tool_name: &str,
    tool_args: &Value,
    approval_id: uuid::Uuid,
    body: &str,
) -> String {
    let header = format!("tool_approval_pending({tool_name}, approval_id={approval_id})");
    format_tool_transcript_with_header(&header, tool_args, body)
}

fn format_tool_transcript_with_header(header: &str, tool_args: &Value, body: &str) -> String {
    let args_json =
        serde_json::to_string_pretty(tool_args).unwrap_or_else(|_| tool_args.to_string());
    format!("{header}:\nargs: {args_json}\n\n{body}")
}

/// True when content already includes the args block from [`format_tool_context_message`].
pub fn tool_context_message_has_args(content: &str) -> bool {
    let trimmed = content.trim_start();
    (trimmed.starts_with("tool_result(")
        || trimmed.starts_with("tool_error(")
        || trimmed.starts_with("tool_approval_pending("))
        && content.contains("\nargs: ")
}

pub fn is_tool_approval_pending_transcript(content: &str) -> bool {
    content.trim_start().starts_with("tool_approval_pending(")
}

/// Raw MCP `pr_get_diff` payload (or the body after a `tool_result` envelope).
pub fn pr_get_diff_raw_output_is_success(output: &str) -> bool {
    let t = output.trim();
    if t.is_empty() {
        return false;
    }
    t.starts_with("Diff for ")
        || t.starts_with("diff --git ")
        || t.starts_with("--- ")
        || t.contains("\ndiff --git ")
        || t.contains("\n--- a/")
}

/// Failure heuristics for raw MCP tool text — header / first line only, not patch hunks.
pub fn tool_body_header_indicates_failure(output: &str) -> bool {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return true;
    }
    if pr_get_diff_raw_output_is_success(trimmed) {
        return false;
    }
    let first_line = trimmed
        .lines()
        .next()
        .unwrap_or(trimmed)
        .to_ascii_lowercase();
    if first_line.starts_with("failed to ") {
        return true;
    }
    if first_line.contains("gateway timeout") {
        return true;
    }
    if first_line.contains("http 504")
        || first_line.contains("http 503")
        || first_line.contains("http 502")
        || first_line.contains("http 500")
    {
        return true;
    }
    if first_line.contains("temporary server error") || first_line.contains("rate limit") {
        return true;
    }
    if first_line.contains("not found") || first_line.contains("http 404") {
        return true;
    }
    false
}

/// Whether a stored tool transcript or legacy plain tool body is a failure.
pub fn tool_transcript_indicates_failure(content: &str) -> bool {
    let trimmed = content.trim_start();
    if trimmed.starts_with("tool_error(") {
        return true;
    }
    if trimmed.starts_with("tool_result(")
        || is_tool_approval_pending_transcript(trimmed)
        || trimmed.starts_with("[tool_result ")
    {
        return false;
    }
    if trimmed.to_ascii_lowercase().starts_with("tool error:") {
        return true;
    }
    tool_body_header_indicates_failure(trimmed)
}

fn pr_number_from_tool_args(args: &Value) -> Option<u32> {
    args.get("pr_number").and_then(|v| {
        v.as_u64()
            .or_else(|| {
                v.as_i64()
                    .filter(|n| *n >= 0)
                    .map(|n| n as u64)
            })
            .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
    }).map(|n| n as u32)
}

fn strip_transcript_args_block(text: &str) -> String {
    let trimmed = text.trim_start();
    if let Some(rest) = trimmed.strip_prefix("args:") {
        if let Some(pos) = rest.find("\n\n") {
            return rest[pos + 2..].trim_start().to_string();
        }
        return String::new();
    }
    trimmed.to_string()
}

pub fn format_history_for_summary(messages: &[LlmTurnMessage]) -> String {
    messages
        .iter()
        .map(|m| format!("[{}]\n{}", m.role, m.content))
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn fit_history_to_budget(messages: &mut Vec<LlmTurnMessage>, token_budget: u32) -> Vec<LlmTurnMessage> {
    let mut dropped = Vec::new();
    while estimate_messages_tokens(messages) > token_budget && messages.len() > 2 {
        let compress_end = messages.len().saturating_sub(2);
        if compress_oldest_tool_in_slice(messages, 0, compress_end) {
            continue;
        }
        dropped.push(messages.remove(0));
    }
    dropped
}

/// Build history turns for the LLM, bounded by message count and token budget (sync / no LLM summary).
pub fn pack_session_history(
    history: &[ChatMessage],
    max_messages: usize,
    token_budget: u32,
) -> Vec<LlmTurnMessage> {
    let take = max_messages.max(2);
    let (dropped_slice, slice) = if history.len() > take {
        let split = history.len() - take;
        (&history[..split], &history[split..])
    } else {
        (&[] as &[ChatMessage], history)
    };

    let mut dropped: Vec<LlmTurnMessage> = dropped_slice
        .iter()
        .map(chat_message_to_llm)
        .collect();
    let mut out: Vec<LlmTurnMessage> = slice.iter().map(chat_message_to_llm).collect();

    dropped.extend(fit_history_to_budget(&mut out, token_budget));

    if !dropped.is_empty() {
        out.insert(
            0,
            LlmTurnMessage::new(
                "user",
                format!(
                    "[{} earlier message(s) omitted from context — full transcript is in the session store]",
                    dropped.len()
                ),
            ),
        );
    }

    out
}

/// Like [`pack_session_history`], but summarizes dropped turns via think=false LLM when enabled.
pub async fn pack_session_history_with_llm(
    history: &[ChatMessage],
    max_messages: usize,
    token_budget: u32,
    llm: &LlmClient,
    compress_history: bool,
    history_summary_min_tokens: u32,
) -> Result<Vec<LlmTurnMessage>> {
    if !compress_history {
        return Ok(pack_session_history(
            history,
            max_messages,
            token_budget,
        ));
    }

    let take = max_messages.max(2);
    let (dropped_slice, slice) = if history.len() > take {
        let split = history.len() - take;
        (&history[..split], &history[split..])
    } else {
        (&[] as &[ChatMessage], history)
    };

    let mut dropped: Vec<LlmTurnMessage> = dropped_slice
        .iter()
        .map(chat_message_to_llm)
        .collect();
    let mut out: Vec<LlmTurnMessage> = slice.iter().map(chat_message_to_llm).collect();

    dropped.extend(fit_history_to_budget(&mut out, token_budget));

    if dropped.is_empty() {
        return Ok(out);
    }

    let dropped_tokens = estimate_messages_tokens(&dropped);
    if dropped_tokens >= history_summary_min_tokens {
        let text = format_history_for_summary(&dropped);
        let summary = llm.summarize_session_history(&text).await?;
        if !summary.trim().is_empty() {
            out.insert(
                0,
                LlmTurnMessage::new("user", format!("[session history summary]\n{summary}")),
            );
            return Ok(out);
        }
    }

    out.insert(
        0,
        LlmTurnMessage::new(
            "user",
            format!(
                "[{} earlier message(s) omitted from context — full transcript is in the session store]",
                dropped.len()
            ),
        ),
    );
    Ok(out)
}

fn chat_message_to_llm(msg: &ChatMessage) -> LlmTurnMessage {
    match msg.role {
        ChatRole::Assistant => {
            if let Some(json) = &msg.tool_calls_json {
                if let Ok(calls) = serde_json::from_str::<Vec<crate::llm::chat::LlmToolCall>>(json) {
                    if !calls.is_empty() {
                        return crate::llm::chat::LlmTurnMessage::assistant_tool_call(
                            msg.content.chars().take(4_000).collect::<String>(),
                            calls,
                        );
                    }
                }
            }
            LlmTurnMessage::new(
                "assistant",
                msg.content.chars().take(4_000).collect::<String>(),
            )
        }
        ChatRole::Tool => {
            let name = msg.tool_name.as_deref().unwrap_or("tool");
            let content = if tool_context_message_has_args(&msg.content) {
                msg.content.clone()
            } else if let Some(args_json) = &msg.tool_calls_json {
                let args = serde_json::from_str(args_json).unwrap_or_else(|_| serde_json::json!({}));
                let ok = !tool_transcript_indicates_failure(&msg.content);
                format_tool_context_message(name, &args, ok, &msg.content)
            } else {
                msg.content.clone()
            };
            LlmTurnMessage::tool_result(name, content)
        }
        ChatRole::User | ChatRole::Harness => LlmTurnMessage::new("user", msg.content.clone()),
    }
}

/// Truncate an oversized system prompt (skill + store snapshot).
pub fn trim_system_content(content: &mut String, max_tokens: u32) {
    let max_chars = (max_tokens as usize).saturating_mul(4);
    if content.chars().count() <= max_chars {
        return;
    }
    *content = truncate_chars(content, max_chars);
    content.push_str("\n\n[system prompt truncated for context budget]");
}

/// Base harness nudge text without the `(Harness retry N …)` suffix.
pub fn harness_nudge_base(content: &str) -> &str {
    content
        .split("\n\n(Harness retry ")
        .next()
        .unwrap_or(content)
}

/// True for harness corrective messages that must survive context trimming.
pub fn is_harness_nudge_content(content: &str) -> bool {
    let trimmed = content.trim_start();
    trimmed.starts_with("[earlier ")
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
        || trimmed.starts_with("Your LLM turn timed out")
        || trimmed.contains("(Harness retry ")
}

fn first_removable_message_index(messages: &[LlmTurnMessage], start: usize, end: usize) -> Option<usize> {
    (start..end).find(|&i| !is_harness_nudge_content(&messages[i].content))
}

/// Shrink `messages` (system at index 0) to fit `token_budget`. Keeps system + recent tail.
pub fn trim_llm_messages(messages: &mut Vec<LlmTurnMessage>, token_budget: u32) {
    const TAIL_PROTECT: usize = 3;
    if messages.len() <= 1 {
        return;
    }
    let mut pass = 0;
    while estimate_messages_tokens(messages) > token_budget && pass < 64 {
        pass += 1;
        let len = messages.len();
        if len <= 1 {
            break;
        }
        let compress_until = len.saturating_sub(TAIL_PROTECT);
        if compress_until > 1
            && compress_oldest_tool_in_slice(messages, 1, compress_until)
        {
            continue;
        }
        if compress_until > 1 {
            if let Some(idx) = first_removable_message_index(messages, 1, compress_until) {
                summarize_message_at(messages, idx);
            }
            if estimate_messages_tokens(messages) <= token_budget {
                break;
            }
        }
        if messages.len() > 1 + TAIL_PROTECT {
            let end = messages.len().saturating_sub(TAIL_PROTECT);
            if let Some(idx) = first_removable_message_index(messages, 1, end) {
                messages.remove(idx);
                continue;
            }
        }
        // Still over budget — compress protected tail (keep last message intact).
        let tail_start = len.saturating_sub(TAIL_PROTECT);
        if tail_start > 0
            && tail_start < len.saturating_sub(1)
            && compress_oldest_tool_in_slice(messages, tail_start, len.saturating_sub(1))
        {
            continue;
        }
        if len > 1 && estimate_messages_tokens(messages) > token_budget {
            let last = messages.len() - 1;
            if llm_message_is_tool_result(&messages[last]) {
                let tool = llm_message_tool_label(&messages[last]);
                let body = if messages[last].role == "tool" {
                    messages[last].content.clone()
                } else {
                    tool_result_body(&messages[last].content)
                };
                messages[last].content = format!(
                    "tool_result({tool}):\n{}",
                    prepare_tool_result_for_context(&tool, &body)
                );
                messages[last].role = "user";
                messages[last].tool_name = None;
            }
        }
        break;
    }
}

fn tool_result_body(content: &str) -> String {
    let after_header = content.lines().skip(1).collect::<Vec<_>>().join("\n");
    strip_transcript_args_block(&after_header)
}

fn compress_oldest_tool_in_slice(
    messages: &mut [LlmTurnMessage],
    start: usize,
    end: usize,
) -> bool {
    for i in start..end.min(messages.len()) {
        if llm_message_is_tool_result(&messages[i]) && !is_already_summarized(&messages[i].content)
        {
            let summary = if messages[i].role == "tool" {
                let tool = llm_message_tool_label(&messages[i]);
                let preview: String = messages[i].content.chars().take(360).collect();
                format!("[summarized tool_result {tool}]\n{preview}…")
            } else {
                summarize_tool_content(&messages[i].content)
            };
            messages[i].content = summary;
            messages[i].role = "user";
            messages[i].tool_name = None;
            return true;
        }
    }
    false
}

fn summarize_message_at(messages: &mut [LlmTurnMessage], idx: usize) {
    if idx >= messages.len() {
        return;
    }
    if is_harness_nudge_content(&messages[idx].content) {
        return;
    }
    let role = messages[idx].role;
    if llm_message_is_tool_result(&messages[idx]) {
        if messages[idx].role == "tool" {
            let tool = llm_message_tool_label(&messages[idx]);
            let preview: String = messages[idx].content.chars().take(360).collect();
            messages[idx].content = format!("[summarized tool_result {tool}]\n{preview}…");
        } else {
            messages[idx].content = summarize_tool_content(&messages[idx].content);
        }
        messages[idx].role = "user";
        messages[idx].tool_name = None;
        return;
    }
    let preview: String = messages[idx].content.chars().take(280).collect();
    messages[idx].content = format!("[earlier {role} message]\n{preview}…");
}

/// True when text is a harness/LLM tool transcript (not a user-facing assistant reply).
pub fn is_tool_result_transcript(content: &str) -> bool {
    let t = content.trim_start();
    t.starts_with("tool_result(")
        || t.starts_with("[tool_result")
        || t.starts_with("tool_error(")
        || is_tool_approval_pending_transcript(content)
        || t.starts_with("[summarized tool_result")
}

fn llm_message_is_tool_result(msg: &LlmTurnMessage) -> bool {
    msg.role == "tool" || is_tool_result_message(&msg.content)
}

fn llm_message_tool_label(msg: &LlmTurnMessage) -> String {
    if let Some(name) = &msg.tool_name {
        return name.clone();
    }
    extract_tool_label(&msg.content).to_string()
}

fn is_tool_result_message(content: &str) -> bool {
    is_tool_result_transcript(content)
}

/// Split `tool_result(name, …): body` into tool name and body.
pub fn split_tool_transcript(content: &str) -> Option<(String, String)> {
    let t = content.trim_start();
    if let Some(rest) = t.strip_prefix("tool_result(") {
        let end = rest.find("):")?;
        let name = rest[..end].split(',').next()?.trim().to_string();
        let body = strip_transcript_args_block(
            rest[end + 2..].trim_start_matches(':').trim_start(),
        );
        return Some((name, body));
    }
    if let Some(rest) = t.strip_prefix("tool_error(") {
        let end = rest.find("):")?;
        let name = rest[..end].split(',').next()?.trim().to_string();
        let body = strip_transcript_args_block(
            rest[end + 2..].trim_start_matches(':').trim_start(),
        );
        return Some((name, body));
    }
    if let Some(rest) = t.strip_prefix("tool_approval_pending(") {
        let end = rest.find("):")?;
        let name = rest[..end].split(',').next()?.trim().to_string();
        let body = strip_transcript_args_block(
            rest[end + 2..].trim_start_matches(':').trim_start(),
        );
        return Some((name, body));
    }
    if let Some(rest) = t.strip_prefix("[tool_result ") {
        let (header, body) = rest.split_once("]\n").or_else(|| rest.split_once("]:"))?;
        let name = header.trim().trim_end_matches(']').to_string();
        let body = body.trim_start_matches('\n').trim_start();
        return Some((name, body.to_string()));
    }
    None
}

fn is_already_summarized(content: &str) -> bool {
    content.starts_with("[summarized tool_result")
        || content.starts_with("[earlier ")
        || content.contains("omitted from context")
}

fn summarize_tool_content(content: &str) -> String {
    let tool = extract_tool_label(content);
    let body = tool_result_body(content);
    let preview: String = body.chars().take(360).collect();
    format!("[summarized tool_result {tool}]\n{preview}…")
}

fn extract_tool_label(content: &str) -> &str {
    if let Some(rest) = content.strip_prefix("tool_result(") {
        if let Some(name) = rest.split(')').next() {
            return name;
        }
    }
    if let Some(rest) = content.strip_prefix("[tool_result ") {
        if let Some(name) = rest.split_whitespace().next() {
            return name.trim_end_matches(']');
        }
    }
    "tool"
}

/// Local fallback when the session-history summarizer LLM is offline or fails.
pub fn truncate_reasoning_local(text: &str, max_chars: usize) -> String {
    let t = text.trim();
    if t.len() <= max_chars {
        return t.to_string();
    }
    format!(
        "{}…\n[reasoning truncated locally to {max_chars} chars]",
        t.chars().take(max_chars).collect::<String>()
    )
}

/// Resolve how many tokens prior session turns may use.
pub fn history_token_budget(budget: &TokenBudget, configured: u32) -> u32 {
    if configured > 0 {
        return configured.min(budget.input_budget() / 2);
    }
    budget.history_budget()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use uuid::Uuid;

    fn sample_history(n: usize) -> Vec<ChatMessage> {
        (0..n)
            .map(|i| ChatMessage {
                id: Uuid::new_v4(),
                session_id: Uuid::new_v4(),
                role: if i % 2 == 0 {
                    ChatRole::User
                } else {
                    ChatRole::Assistant
                },
                content: format!("message {i} {}", "x".repeat(200)),
                tool_name: None,
                tool_calls_json: None,
                ts: Utc::now(),
            })
            .collect()
    }

    #[test]
    fn format_context_panel_shows_native_tool_calls() {
        use crate::llm::chat::LlmToolCall;
        let msg = LlmTurnMessage::assistant_tool_call(
            "",
            vec![LlmToolCall {
                id: "call_1".into(),
                name: "pr_get_overview".into(),
                arguments: serde_json::json!({"repo": "acme/widget", "pr_number": 42}),
            }],
        );
        let text = format_llm_message_for_context_panel(&msg);
        assert!(text.contains("tool_call: pr_get_overview"));
        assert!(text.contains("args: {"));
        assert!(text.contains("acme/widget"));
        assert!(estimate_message_tokens(&msg) > 4);
    }

    #[test]
    fn split_tool_transcript_parses_header() {
        let raw = "tool_result(pr_list_changed_files, pr_number=19275):\n1 changed file(s)";
        let (name, body) = split_tool_transcript(raw).expect("parsed");
        assert_eq!(name, "pr_list_changed_files");
        assert!(body.contains("1 changed file"));
    }

    #[test]
    fn format_tool_context_message_includes_args() {
        let args = serde_json::json!({
            "repo": "acme/widget",
            "pr_number": 19853,
            "max_bytes": 32000
        });
        let text = format_tool_context_message(
            "pr_get_diff",
            &args,
            true,
            "Diff for acme/widget#19853 (10 bytes):\n\n+line",
        );
        assert!(text.starts_with("tool_result(pr_get_diff, pr_number=19853):"));
        assert!(text.contains("args:"));
        assert!(text.contains("max_bytes"));
        let (_, body) = split_tool_transcript(&text).expect("parsed");
        assert!(body.starts_with("Diff for"));
    }

    #[test]
    fn format_tool_approval_pending_is_not_tool_error() {
        let args = serde_json::json!({ "repo": "acme/widget", "run_id": 42 });
        let text = format_tool_approval_pending_message(
            "ci_rerun_workflow",
            &args,
            uuid::Uuid::nil(),
            "Mutating tool awaiting approval.",
        );
        assert!(text.starts_with("tool_approval_pending(ci_rerun_workflow"));
        assert!(!text.starts_with("tool_error("));
        assert!(is_tool_result_transcript(&text));
        assert!(is_tool_approval_pending_transcript(&text));
    }

    #[test]
    fn tool_transcript_success_ignores_failed_to_in_diff_body() {
        let stored = format_tool_context_message(
            "pr_get_diff",
            &serde_json::json!({"repo": "acme/widget", "pr_number": 1}),
            true,
            "Diff for acme/widget#1 (80 bytes):\n\n\
diff --git a/err.go b/err.go\n\
+  return fmt.Errorf(\"failed to open file\")",
        );
        assert!(!tool_transcript_indicates_failure(&stored));
    }

    #[test]
    fn tool_transcript_failure_uses_tool_error_prefix() {
        let stored = format_tool_context_message(
            "pr_get_diff",
            &serde_json::json!({"repo": "acme/widget", "pr_number": 1}),
            false,
            "failed to fetch PR diff: HTTP 404",
        );
        assert!(tool_transcript_indicates_failure(&stored));
    }

    #[test]
    fn is_tool_result_transcript_detects_harness_format() {
        assert!(is_tool_result_transcript(
            "tool_result(pr_get_overview, pr_number=1):\nbody"
        ));
        assert!(!is_tool_result_transcript("Here is the PR summary."));
    }

    #[test]
    fn structure_pr_get_diff_splits_by_file() {
        let raw = "Diff for o/r#1 (100 bytes):\n\n\
diff --git a/foo.rs b/foo.rs\n\
--- a/foo.rs\n+++ b/foo.rs\n\
+line one\n\
diff --git a/bar.rs b/bar.rs\n\
--- a/bar.rs\n+++ b/bar.rs\n\
+line two\n";
        let structured = structure_tool_result("pr_get_diff", raw);
        assert!(structured.contains("### foo.rs"));
        assert!(structured.contains("### bar.rs"));
        assert!(structured.contains("+line one"));
    }

    #[test]
    fn chat_message_harness_roundtrips_to_llm_user_turn() {
        use crate::store::{ChatMessage, ChatRole};
        use chrono::Utc;
        use uuid::Uuid;
        let msg = ChatMessage {
            id: Uuid::new_v4(),
            session_id: Uuid::new_v4(),
            role: ChatRole::Harness,
            content: "Tool `pr_get_overview` is missing required `repo`.".into(),
            ts: Utc::now(),
            tool_name: None,
            tool_calls_json: None,
        };
        let llm = chat_message_to_llm(&msg);
        assert_eq!(llm.role, "user");
        assert!(llm.content.contains("missing required `repo`"));
    }

    #[test]
    fn trim_preserves_harness_nudge_content() {
        use crate::agent::tool_catalog::ToolCatalog;
        use crate::llm::LlmTurnMessage;
        let nudge = ToolCatalog::full()
            .format_tool_args_nudge("pr_get_overview", "pr_number", Some("19272"), Some("acme/widget"));
        let mut messages = vec![
            LlmTurnMessage::new("system", "sys".repeat(5000)),
            LlmTurnMessage::new("user", "user".repeat(5000)),
            LlmTurnMessage::new("user", nudge.clone()),
        ];
        trim_llm_messages(&mut messages, 200);
        assert!(
            messages.iter().any(|m| m.content.contains("Tool `pr_get_overview`")),
            "harness nudge must survive trimming"
        );
        assert!(messages.last().is_some_and(|m| m.content.contains("19272")));
    }

    #[test]
    fn estimate_tokens_nonzero() {
        assert!(estimate_tokens("hello") >= 1);
    }

    #[test]
    fn list_tool_gets_smaller_cap() {
        assert!(tool_result_char_cap("pr_list_open") < tool_result_char_cap("pr_get_status"));
    }

    #[test]
    fn structure_pr_list_keeps_number_and_ci() {
        let raw = "open PR(s) in o/r (3):\n\
#19235  backport title  @alice  CI:failing  review:approved\n\
#19240  other  @bob  CI:passing  review:none";
        let structured = structure_tool_result("pr_list_open", raw);
        assert!(structured.contains("#19235 CI:failing"), "got: {structured}");
        assert!(structured.contains("#19240 CI:passing"));
        assert!(!structured.contains("@alice"));
    }

    #[test]
    fn tool_results_are_not_focus_filtered() {
        let unrelated = prepare_tool_result_for_context(
            "pr_get_status",
            "PR #9999 merged",
        );
        assert!(unrelated.contains("#9999"));
        assert!(!unrelated.contains("omitted"));

        let related = prepare_tool_result_for_context(
            "pr_get_overview",
            "PR #19235 overview",
        );
        assert!(related.contains("19235"));
    }

    #[test]
    fn list_tools_keep_structured_output() {
        let list = prepare_tool_result_for_context(
            "pr_list_open",
            "#1  a  @x  CI:passing  review:none\n#2  b  @y  CI:failing  review:none",
        );
        assert!(!list.contains("omitted"));
        assert!(list.contains("#1 CI:passing"));
    }

    #[test]
    fn pack_history_respects_token_budget() {
        let history = sample_history(20);
        let budget = TokenBudget::from_config(64_000);
        let packed = pack_session_history(&history, 20, budget.history_budget() / 4);
        assert!(estimate_messages_tokens(&packed) <= budget.history_budget() / 4 + 500);
    }

    #[test]
    fn trim_keeps_system_and_tail() {
        let mut msgs = vec![LlmTurnMessage::new("system", "sys")];
        for _ in 0..10 {
            msgs.push(LlmTurnMessage::new(
                "user",
                format!("tool_result(pr_list_open):\n{}", "y".repeat(3000)),
            ));
        }
        trim_llm_messages(&mut msgs, 900);
        assert_eq!(msgs[0].role, "system");
        assert!(msgs.len() >= 2);
        assert!(estimate_messages_tokens(&msgs) <= 950);
    }

    #[test]
    fn truncate_chars_respects_unicode_boundaries() {
        let msg = "你只需要给这些所有的 PR 的 changed files 总结一下，其他的不需要做。";
        let out = truncate_chars(msg, 20);
        assert!(out.ends_with('…'));
        assert!(std::str::from_utf8(out.as_bytes()).is_ok());
        assert!(out.chars().count() <= 21);
    }
}
