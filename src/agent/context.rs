//! Token-aware chat context packing for 64K (and other) context windows.

use std::collections::HashMap;

use crate::agent::budget::TokenBudget;
use crate::agent::parse::{parse_issue_line, parse_pr_line};
use crate::error::Result;
use crate::llm::{LlmClient, LlmTurnMessage};
use crate::store::{ChatMessage, ChatRole};
use serde_json::{json, Value};

/// Rough token estimate (~4 chars per token for Latin/mixed text).
pub fn estimate_tokens(text: &str) -> u32 {
    (text.len() as u32 / 4).max(1)
}

pub fn estimate_messages_tokens(messages: &[LlmTurnMessage]) -> u32 {
    messages.iter().map(estimate_message_tokens).sum()
}

/// Rough token estimate for native `tools[]` JSON attached to a chat step.
pub fn estimate_tools_tokens(tools: &[Value]) -> u32 {
    if tools.is_empty() {
        return 0;
    }
    let json = serde_json::to_string(tools).unwrap_or_default();
    estimate_tokens(&json).saturating_add((tools.len() as u32).saturating_mul(8))
}

pub fn tool_names_from_definitions(tools: &[Value]) -> Vec<String> {
    tools
        .iter()
        .filter_map(|d| {
            d.get("function")
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str())
                .map(str::to_string)
        })
        .collect()
}

/// Message trim budget after reserving estimated tool-schema tokens.
pub fn message_budget_for_tools(input_budget: u32, tools: &[Value]) -> u32 {
    const MIN_MESSAGE_BUDGET: u32 = 2048;
    input_budget
        .saturating_sub(estimate_tools_tokens(tools))
        .max(MIN_MESSAGE_BUDGET)
}

const CONTEXT_PANEL_SECTION_MAX_CHARS: usize = 48_000;

/// Human-readable native tool schemas for the LLM Context panel.
pub fn format_tools_for_context_panel(tools: &[Value]) -> String {
    if tools.is_empty() {
        return "(no tools exposed on this step)".into();
    }
    let mut parts = Vec::new();
    let mut total = 0usize;
    for t in tools {
        let Some(func) = t.get("function") else {
            continue;
        };
        let name = func.get("name").and_then(|n| n.as_str()).unwrap_or("?");
        let desc = func
            .get("description")
            .and_then(|d| d.as_str())
            .unwrap_or("");
        let params = func.get("parameters").cloned().unwrap_or_else(|| json!({}));
        let params_str = serde_json::to_string_pretty(&params).unwrap_or_else(|_| "{}".to_string());
        let block = format!("### {name}\n{desc}\n\n```json\n{params_str}\n```");
        total = total.saturating_add(block.len());
        if total > CONTEXT_PANEL_SECTION_MAX_CHARS {
            parts.push(
                "[remaining tool schemas omitted from display — still sent to the LLM]".into(),
            );
            break;
        }
        parts.push(block);
    }
    parts.join("\n\n")
}

/// System prompt body for the panel — techniques are shown under `[skill]` blocks.
pub fn format_system_for_context_panel(content: &str) -> String {
    const TECH: &str = "\n## Techniques\n";
    const TOOLS: &str = "\n## Tools\n";
    const CTX: &str = "\n## Context\n";
    if let Some(tech_start) = content.find(TECH) {
        let before = &content[..tech_start];
        if let Some(ctx_start) = content.find(CTX) {
            return format!("{before}{CTX}{}", &content[ctx_start + CTX.len()..])
                .trim()
                .to_string();
        }
        return before.trim().to_string();
    }
    if let Some(tools_start) = content.find(TOOLS) {
        let before = &content[..tools_start];
        if let Some(ctx_start) = content.find(CTX) {
            return format!("{before}{CTX}{}", &content[ctx_start + CTX.len()..])
                .trim()
                .to_string();
        }
    }
    content.trim().to_string()
}

pub fn skill_body_for_context_panel(body: &str) -> String {
    crate::engine::skill::skill_body_for_prompt(body)
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
    let prose = strip_reasoning_summary_marker(&msg.content);
    let mut parts = Vec::new();
    if !prose.trim().is_empty() {
        parts.push(prose.to_string());
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

/// Compaction strategy for long chat sessions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactionStrategy {
    Code,
    Ops,
    Generic,
}

pub fn tool_result_char_cap(strategy: CompactionStrategy, tool_name: &str) -> usize {
    match strategy {
        CompactionStrategy::Code => coding_tool_result_char_cap(tool_name),
        CompactionStrategy::Ops | CompactionStrategy::Generic => {
            ops_tool_result_char_cap(tool_name)
        }
    }
}

/// Per-tool byte cap when compressing older tool turns (not the live turn path).
pub fn cap_tool_result_for_strategy(
    strategy: CompactionStrategy,
    tool_name: &str,
    text: &str,
) -> String {
    cap_tool_result_with_cap(tool_result_char_cap(strategy, tool_name), tool_name, text)
}

fn coding_tool_result_char_cap(tool_name: &str) -> usize {
    match tool_name {
        "read_file" => 4_000,
        "grep" => 3_500,
        "glob" => 2_500,
        "bash_run" => 3_000,
        "python_run" => 3_000,
        "web_fetch" => 4_000,
        "edit_file" | "write_file" => 1_200,
        _ => ops_tool_result_char_cap(tool_name),
    }
}

fn ops_tool_result_char_cap(tool_name: &str) -> usize {
    match tool_name {
        "pr_list_open" | "pr_list_merged" | "pr_list_waiting_review" | "issue_list_open" => 4_200,
        "ci_get_failed_logs" => 7_200,
        "pr_list_changed_files" => 4_800,
        "pr_get_diff" => 6_000,
        "pr_get_overview"
        | "pr_get_status"
        | "pr_get_status_batch"
        | "ci_analyze_pr_failures"
        | "ci_get_run_summary"
        | "ci_failure_fingerprint" => 5_250,
        "ci_compare_runs" | "ci_list_external_checks" => 3_000,
        "repo_get_info" => 2_500,
        "store_get_latest_digest" | "store_list_pending_approvals" => 3_000,
        "store_get_oncall_handoff" => 6_500,
        _ => 9_000,
    }
}

fn summarize_tool_result_for_compaction(
    strategy: CompactionStrategy,
    tool_name: &str,
    content: &str,
) -> String {
    match strategy {
        CompactionStrategy::Code => summarize_coding_tool_content(tool_name, content),
        CompactionStrategy::Ops => summarize_ops_tool_content(tool_name, content),
        CompactionStrategy::Generic => summarize_tool_content(content),
    }
}

fn summarize_coding_tool_content(tool_name: &str, content: &str) -> String {
    match tool_name {
        "bash_run" => summarize_bash_run_for_compaction(content),
        "python_run" => summarize_python_run_for_compaction(content),
        "web_fetch" => summarize_web_fetch_for_compaction(content),
        "read_file" | "grep" => {
            let preview: String = content.chars().take(800).collect();
            format!("[summarized tool_result {tool_name}]\n{preview}…")
        }
        _ => summarize_tool_content(content),
    }
}

fn summarize_bash_run_for_compaction(content: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let exit = lines
        .iter()
        .find(|l| l.trim().starts_with("exit:"))
        .copied()
        .unwrap_or("exit: ?");
    let tail: Vec<&str> = lines.iter().rev().take(20).copied().collect();
    let mut tail: Vec<&str> = tail.into_iter().rev().collect();
    if tail.is_empty() {
        tail = lines.iter().take(20).copied().collect();
    }
    format!(
        "[summarized tool_result bash_run]\n{exit}\n{}",
        tail.join("\n")
    )
}

fn summarize_python_run_for_compaction(content: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let exit = lines
        .iter()
        .find(|l| l.trim().starts_with("exit:"))
        .copied()
        .unwrap_or("exit: ?");
    let tail: Vec<&str> = lines.iter().rev().take(20).copied().collect();
    let mut tail: Vec<&str> = tail.into_iter().rev().collect();
    if tail.is_empty() {
        tail = lines.iter().take(20).copied().collect();
    }
    format!(
        "[summarized tool_result python_run]\n{exit}\n{}",
        tail.join("\n")
    )
}

fn summarize_web_fetch_for_compaction(content: &str) -> String {
    let mut header = Vec::new();
    let mut in_body = false;
    let mut body_lines = Vec::new();
    for line in content.lines() {
        if line.trim() == "---" {
            in_body = true;
            continue;
        }
        if in_body {
            body_lines.push(line);
        } else {
            let t = line.trim();
            if t.starts_with("web_fetch:")
                || t.starts_with("web_browser:")
                || t.starts_with("status:")
                || t.starts_with("content-type:")
                || t.starts_with("title:")
                || t.starts_with("description:")
                || t.starts_with("headings:")
                || t.starts_with("links:")
                || t.starts_with("warning:")
                || t.starts_with("- ")
            {
                header.push(t.to_string());
            }
        }
    }
    let tail: String = body_lines
        .iter()
        .rev()
        .take(12)
        .copied()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n");
    let mut out = format!("[summarized tool_result web_fetch]\n{}", header.join("\n"));
    if !tail.is_empty() {
        out.push_str("\n---\n");
        out.push_str(&tail);
        if body_lines.len() > 12 {
            out.push('…');
        }
    }
    out
}

fn summarize_ops_tool_content(tool_name: &str, content: &str) -> String {
    let body = if content.trim_start().starts_with("tool_result(")
        || content.trim_start().starts_with("[tool_result ")
    {
        tool_result_body(content)
    } else {
        content.to_string()
    };
    let critical = extract_ops_critical_lines(&body);
    let mut parts = vec![format!("[summarized tool_result {tool_name}]")];
    if !critical.is_empty() {
        parts.push(critical.join("\n"));
    } else {
        let preview: String = body.chars().take(TOOL_SUMMARY_PREVIEW_CHARS).collect();
        parts.push(format!("{preview}…"));
    }
    parts.join("\n")
}

/// Lines to keep when compacting ops / MCP tool output.
pub fn extract_ops_critical_lines(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        let lower = t.to_ascii_lowercase();
        let keep = lower.contains("ci_kind")
            || lower.contains("verdict")
            || lower.contains("flaky")
            || lower.contains("policy")
            || lower.starts_with("error:")
            || lower.contains("needs_attention")
            || lower.contains("digest")
            || (t.contains('#') && t.contains('/'))
            || (t.contains("PR #") || t.contains("pr #"))
            || lower.contains("triage:")
            || lower.contains("failing run")
            || lower.contains("workflow:");
        if keep {
            out.push(t.to_string());
        }
    }
    out.sort();
    out.dedup();
    out
}

pub fn cap_tool_result(tool_name: &str, text: &str) -> String {
    cap_tool_result_for_strategy(CompactionStrategy::Ops, tool_name, text)
}

fn cap_tool_result_with_cap(cap: usize, _tool_name: &str, text: &str) -> String {
    if text.chars().count() <= cap {
        return text.to_string();
    }
    let critical: Vec<String> = text
        .lines()
        .filter(|line| {
            let t = line.trim();
            t.starts_with("ERROR:")
                || t.starts_with("OK:")
                || t.starts_with("PAGE:")
                || t.starts_with("External checks")
                || (t.chars().next().is_some_and(|c| c.is_ascii_digit())
                    && t.contains("  ")
                    && !t.starts_with("20"))
        })
        .map(str::to_string)
        .collect();
    let body = truncate_chars(text, cap);
    if critical.is_empty() {
        return format!(
            "{}…\n[truncated {} chars — use a narrower tool or follow-up for full output]",
            body,
            text.chars().count().saturating_sub(cap)
        );
    }
    format!(
        "{}\n\n{}…\n[truncated {} chars]",
        critical.join("\n"),
        body,
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
    // Single-file fetch already targets one path; keep the patch intact (cap only).
    if text.contains(" path=") {
        return cap_tool_result("pr_get_diff", text);
    }

    const MAX_FILES: usize = 16;
    const PER_FILE_CHARS: usize = 1_200;

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
            "\n… and {} more file(s) — call pr_list_changed_files, then pr_get_diff with path=<file>",
            files.len() - MAX_FILES
        ));
    }
    if text.contains("[diff truncated at max_bytes]") {
        out.push_str(
            "\n[upstream diff truncated at max_bytes — use pr_get_diff with path=<file> per file]",
        );
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
    let first_line = trimmed.lines().next().unwrap_or(trimmed).trim();
    let first_lower = first_line.to_ascii_lowercase();
    if first_lower.starts_with("ok:") {
        return false;
    }
    if first_lower.starts_with("error:") {
        return true;
    }
    if first_lower.starts_with("page:") {
        return false;
    }
    if first_lower.starts_with("failed to ") {
        return true;
    }
    if first_line.contains("gateway timeout") {
        return true;
    }
    if first_lower.contains("http 504")
        || first_lower.contains("http 503")
        || first_lower.contains("http 502")
        || first_lower.contains("http 500")
    {
        return true;
    }
    if first_lower.contains("temporary server error") || first_lower.contains("rate limit") {
        return true;
    }
    if first_lower.contains("not found") || first_lower.contains("http 404") {
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
    args.get("pr_number")
        .and_then(|v| {
            v.as_u64()
                .or_else(|| v.as_i64().filter(|n| *n >= 0).map(|n| n as u64))
                .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
        })
        .map(|n| n as u32)
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

fn fit_history_to_budget(
    messages: &mut Vec<LlmTurnMessage>,
    token_budget: u32,
) -> Vec<LlmTurnMessage> {
    const TAIL_KEEP: usize = 4;
    let mut dropped = Vec::new();
    while estimate_messages_tokens(messages) > token_budget && messages.len() > TAIL_KEEP {
        let compress_end = messages.len().saturating_sub(TAIL_KEEP);
        if compress_oldest_tool_in_slice(messages, 0, compress_end, CompactionStrategy::Generic) {
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

    let mut dropped: Vec<LlmTurnMessage> = dropped_slice.iter().map(chat_message_to_llm).collect();
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
    compaction: CompactionStrategy,
) -> Result<Vec<LlmTurnMessage>> {
    if !compress_history {
        return Ok(pack_session_history(history, max_messages, token_budget));
    }

    let take = max_messages.max(2);
    let (dropped_slice, slice) = if history.len() > take {
        let split = history.len() - take;
        (&history[..split], &history[split..])
    } else {
        (&[] as &[ChatMessage], history)
    };

    let mut dropped: Vec<LlmTurnMessage> = dropped_slice.iter().map(chat_message_to_llm).collect();
    let mut out: Vec<LlmTurnMessage> = slice.iter().map(chat_message_to_llm).collect();

    dropped.extend(fit_history_to_budget(&mut out, token_budget));

    if dropped.is_empty() {
        return Ok(out);
    }

    let dropped_tokens = estimate_messages_tokens(&dropped);
    if dropped_tokens >= history_summary_min_tokens {
        let text = format_history_for_summary(&dropped);
        let summary = summarize_history_batch(llm, compaction, &text).await?;
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

pub fn chat_message_to_llm(msg: &ChatMessage) -> LlmTurnMessage {
    match msg.role {
        ChatRole::Assistant => {
            if let Some(json) = &msg.tool_calls_json {
                if let Ok(calls) = serde_json::from_str::<Vec<crate::llm::chat::LlmToolCall>>(json)
                {
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
                let args =
                    serde_json::from_str(args_json).unwrap_or_else(|_| serde_json::json!({}));
                let ok = !tool_transcript_indicates_failure(&msg.content);
                format_tool_context_message(name, &args, ok, &msg.content)
            } else {
                msg.content.clone()
            };
            LlmTurnMessage::tool_result(name, content)
        }
        ChatRole::User | ChatRole::Harness | ChatRole::Reasoning => {
            LlmTurnMessage::new("user", msg.content.clone())
        }
    }
}

/// Truncate an oversized system prompt — drop bulky sections before blind tail cut.
pub fn trim_system_content(content: &mut String, max_tokens: u32) {
    let max_chars = (max_tokens as usize).saturating_mul(4);
    if content.chars().count() <= max_chars {
        return;
    }
    // NOTE: "Available skills" is no longer in the system prompt (it's a
    // separate message), so we only drop Techniques and Tools here.
    const TECH: &str = "\n## Techniques\n";
    const TOOLS: &str = "\n## Tools\n";
    if let Some(tech_start) = content.find(TECH) {
        let before = content[..tech_start].to_string();
        if before.chars().count() <= max_chars {
            *content = before;
            content.push_str("\n\n[Techniques omitted for context budget — use skill_load]");
            return;
        }
    }
    if let Some(tools_start) = content.find(TOOLS) {
        let before = content[..tools_start].to_string();
        if before.chars().count() <= max_chars {
            *content = before;
            content.push_str("\n\n[Tools section omitted for context budget]");
            return;
        }
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
        || trimmed.starts_with("HARN:TOOL_FAILED")
        || trimmed.contains("[Harness]")
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

fn first_removable_message_index(
    messages: &[LlmTurnMessage],
    start: usize,
    end: usize,
) -> Option<usize> {
    (start..end).find(|&i| !is_context_protected_content(&messages[i].content))
}

/// Chars kept when summarizing an older tool turn during trim (live turn stays full).
const TOOL_SUMMARY_PREVIEW_CHARS: usize = 1_200;

/// Prior rolling summaries produced when context exceeded the budget.
pub fn is_rolling_summary_content(content: &str) -> bool {
    let t = content.trim_start();
    t.starts_with("[session history summary]")
        || t.starts_with("[earlier context summary]")
        || (t.starts_with("[earlier ") && t.contains("omitted from context"))
}

/// Parse `[N earlier message(s) omitted from context …]` markers.
fn parse_omitted_message_count(content: &str) -> Option<u32> {
    let rest = content.strip_prefix('[')?;
    let num: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if num.is_empty() {
        return None;
    }
    if !rest[num.len()..].starts_with(" earlier message") {
        return None;
    }
    num.parse().ok()
}

/// Trim/summary signals for the context panel (derived from LLM message markers).
pub fn analyze_context_trim_metadata(messages: &[LlmTurnMessage]) -> (u32, Option<String>) {
    let mut omitted_turns = 0u32;
    let mut session_summary = false;
    let mut context_summaries = 0u32;
    let mut summarized_tools = 0u32;

    for m in messages {
        let t = m.content.trim_start();
        if let Some(n) = parse_omitted_message_count(t) {
            omitted_turns = omitted_turns.saturating_add(n);
        } else if t.starts_with("[session history summary]") {
            session_summary = true;
        } else if t.starts_with("[earlier context summary]") {
            context_summaries += 1;
        } else if t.starts_with("[summarized tool_result") {
            summarized_tools += 1;
        }
    }

    let mut parts = Vec::new();
    if omitted_turns > 0 {
        parts.push(format!(
            "{omitted_turns} earlier turn{} omitted",
            if omitted_turns == 1 { "" } else { "s" }
        ));
    }
    if session_summary {
        parts.push("session history summarized".to_string());
    }
    if context_summaries > 0 {
        parts.push(if context_summaries == 1 {
            "earlier turns summarized".to_string()
        } else {
            format!("{context_summaries} earlier turn blocks summarized")
        });
    }
    if summarized_tools > 0 {
        parts.push(format!(
            "{summarized_tools} tool output{} summarized",
            if summarized_tools == 1 { "" } else { "s" }
        ));
    }

    let note = if parts.is_empty() {
        None
    } else {
        Some(parts.join(" · "))
    };
    (omitted_turns, note)
}

/// Compressed thinking trace injected after a long reasoning stream.
pub fn is_reasoning_summary_content(content: &str) -> bool {
    content
        .trim_start()
        .starts_with("[agent reasoning summary]")
}

/// Hide the harness marker in the context panel — the `[reasoning]` role label is enough.
pub fn strip_reasoning_summary_marker(content: &str) -> &str {
    let trimmed = content.trim_start();
    let Some(rest) = trimmed.strip_prefix("[agent reasoning summary]") else {
        return content;
    };
    rest.trim_start_matches('\n')
}

/// Map LLM reasoning message content → raw thinking trace, from persisted history.
pub fn reasoning_originals_from_history(history: &[ChatMessage]) -> HashMap<String, String> {
    history
        .iter()
        .filter(|m| m.role == ChatRole::Reasoning)
        .filter_map(|m| {
            m.reasoning_original
                .as_ref()
                .map(|orig| (m.content.clone(), orig.clone()))
        })
        .collect()
}

fn is_context_protected_content(content: &str) -> bool {
    is_harness_nudge_content(content)
        || is_rolling_summary_content(content)
        || is_reasoning_summary_content(content)
}

fn collapsible_indices_for_summary(messages: &[LlmTurnMessage], tail_protect: usize) -> Vec<usize> {
    let collapse_end = messages.len().saturating_sub(tail_protect);
    (1..collapse_end)
        .filter(|&i| !is_context_protected_content(&messages[i].content))
        .collect()
}

/// Batch older turns into one LLM summary message (harness nudges stay in place).
async fn try_collapse_old_messages_with_llm(
    messages: &mut Vec<LlmTurnMessage>,
    token_budget: u32,
    llm: &LlmClient,
    summary_min_tokens: u32,
    compaction: CompactionStrategy,
) -> Result<()> {
    const TAIL_PROTECT: usize = 8;
    const MAX_PASSES: u32 = 4;

    for _ in 0..MAX_PASSES {
        if estimate_messages_tokens(messages) <= token_budget {
            return Ok(());
        }
        let indices = collapsible_indices_for_summary(messages, TAIL_PROTECT);
        if indices.len() < 2 {
            break;
        }
        let batch: Vec<LlmTurnMessage> = indices.iter().map(|&i| messages[i].clone()).collect();
        let batch_tokens = estimate_messages_tokens(&batch);
        if batch_tokens < summary_min_tokens && indices.len() < 3 {
            break;
        }
        let text = format_history_for_summary(&batch);
        let summary = summarize_history_batch(llm, compaction, &text).await?;
        if summary.trim().is_empty() {
            break;
        }
        let insert_at = indices[0];
        for &i in indices.iter().rev() {
            messages.remove(i);
        }
        messages.insert(
            insert_at,
            LlmTurnMessage::new("user", format!("[earlier context summary]\n{summary}")),
        );
    }
    Ok(())
}

async fn summarize_history_batch(
    llm: &LlmClient,
    compaction: CompactionStrategy,
    text: &str,
) -> Result<String> {
    if compaction == CompactionStrategy::Ops {
        let preserved = extract_ops_critical_lines(text);
        if preserved.len() >= 2 {
            return Ok(format!("Preserved ops facts:\n{}", preserved.join("\n")));
        }
    }
    match compaction {
        CompactionStrategy::Code => llm.summarize_coding_session_history(text).await,
        CompactionStrategy::Ops => llm.summarize_ops_session_history(text).await,
        CompactionStrategy::Generic => llm.summarize_session_history(text).await,
    }
}

/// Fit messages to the token budget; older turns may be rolled into one LLM summary first.
pub async fn trim_llm_messages_with_llm(
    messages: &mut Vec<LlmTurnMessage>,
    token_budget: u32,
    llm: &LlmClient,
    compress_with_llm: bool,
    summary_min_tokens: u32,
    compaction: CompactionStrategy,
) -> Result<()> {
    if compress_with_llm && estimate_messages_tokens(messages) > token_budget {
        try_collapse_old_messages_with_llm(
            messages,
            token_budget,
            llm,
            summary_min_tokens,
            compaction,
        )
        .await?;
    }
    if estimate_messages_tokens(messages) > token_budget {
        trim_llm_messages(messages, token_budget, compaction);
    }
    Ok(())
}

/// Shrink `messages` (system at index 0) to fit `token_budget`. Keeps system + recent tail.
pub fn trim_llm_messages(
    messages: &mut Vec<LlmTurnMessage>,
    token_budget: u32,
    compaction: CompactionStrategy,
) {
    const TAIL_PROTECT: usize = 8;
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
            && compress_oldest_tool_in_slice(messages, 1, compress_until, compaction)
        {
            continue;
        }
        if compress_until > 1 {
            if let Some(idx) = first_removable_message_index(messages, 1, compress_until) {
                summarize_message_at(messages, idx, compaction);
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
            && compress_oldest_tool_in_slice(
                messages,
                tail_start,
                len.saturating_sub(1),
                compaction,
            )
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
    compaction: CompactionStrategy,
) -> bool {
    for i in start..end.min(messages.len()) {
        if llm_message_is_tool_result(&messages[i])
            && !is_already_summarized(&messages[i].content)
            && !is_rolling_summary_content(&messages[i].content)
        {
            let summary = if messages[i].role == "tool" {
                let tool = llm_message_tool_label(&messages[i]);
                summarize_tool_result_for_compaction(compaction, &tool, &messages[i].content)
            } else {
                match compaction {
                    CompactionStrategy::Ops => {
                        summarize_ops_tool_content("tool", &messages[i].content)
                    }
                    _ => summarize_tool_content(&messages[i].content),
                }
            };
            messages[i].content = summary;
            messages[i].role = "user";
            messages[i].tool_name = None;
            return true;
        }
    }
    false
}

fn summarize_message_at(
    messages: &mut [LlmTurnMessage],
    idx: usize,
    compaction: CompactionStrategy,
) {
    if idx >= messages.len() {
        return;
    }
    if is_context_protected_content(&messages[idx].content) {
        return;
    }
    let role = messages[idx].role;
    if llm_message_is_tool_result(&messages[idx]) {
        if messages[idx].role == "tool" {
            let tool = llm_message_tool_label(&messages[idx]);
            messages[idx].content =
                summarize_tool_result_for_compaction(compaction, &tool, &messages[idx].content);
        } else {
            messages[idx].content = match compaction {
                CompactionStrategy::Ops => {
                    summarize_ops_tool_content("tool", &messages[idx].content)
                }
                _ => summarize_tool_content(&messages[idx].content),
            };
        }
        messages[idx].role = "user";
        messages[idx].tool_name = None;
        return;
    }
    let preview: String = messages[idx].content.chars().take(500).collect();
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
        let body =
            strip_transcript_args_block(rest[end + 2..].trim_start_matches(':').trim_start());
        return Some((name, body));
    }
    if let Some(rest) = t.strip_prefix("tool_error(") {
        let end = rest.find("):")?;
        let name = rest[..end].split(',').next()?.trim().to_string();
        let body =
            strip_transcript_args_block(rest[end + 2..].trim_start_matches(':').trim_start());
        return Some((name, body));
    }
    if let Some(rest) = t.strip_prefix("tool_approval_pending(") {
        let end = rest.find("):")?;
        let name = rest[..end].split(',').next()?.trim().to_string();
        let body =
            strip_transcript_args_block(rest[end + 2..].trim_start_matches(':').trim_start());
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
    let preview: String = body.chars().take(TOOL_SUMMARY_PREVIEW_CHARS).collect();
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
        return configured.min(budget.input_budget() * 3 / 4);
    }
    budget.history_budget()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use uuid::Uuid;

    #[test]
    fn format_system_for_context_panel_strips_techniques() {
        let raw = "Agent\n\n## Techniques\n### ci\nrules\n\n## Context\nrepos: x";
        let out = format_system_for_context_panel(raw);
        assert!(out.contains("Agent"));
        assert!(out.contains("repos: x"));
        assert!(!out.contains("Techniques"));
        assert!(!out.contains("### ci"));
    }

    #[test]
    fn trim_system_content_drops_techniques_before_blind_cut() {
        // Available skills is no longer in the system prompt (separate message),
        // so trim_system_content only drops Techniques and Tools.
        let mut content = format!(
            "{}\n\n## Techniques\n{}",
            "Agent body ".repeat(200),
            "skill ".repeat(5000),
        );
        trim_system_content(&mut content, 800);
        assert!(!content.contains("## Techniques"));
        assert!(content.contains("skill_load"));
    }

    #[test]
    fn format_tools_for_context_panel_includes_schema() {
        let tools = vec![json!({
            "type": "function",
            "function": {
                "name": "tool_search",
                "description": "search tools",
                "parameters": { "type": "object", "required": ["query"] }
            }
        })];
        let text = format_tools_for_context_panel(&tools);
        assert!(text.contains("tool_search"));
        assert!(text.contains("search tools"));
        assert!(text.contains("query"));
    }

    #[test]
    fn estimate_tools_tokens_counts_json_payload() {
        let tools = vec![serde_json::json!({
            "type": "function",
            "function": { "name": "tool_search", "description": "search", "parameters": {} }
        })];
        assert!(estimate_tools_tokens(&tools) > 10);
    }

    #[test]
    fn message_budget_for_tools_subtracts_tool_estimate() {
        let tools = vec![serde_json::json!({
            "type": "function",
            "function": { "name": "x", "description": "y", "parameters": {} }
        })];
        let tools_t = estimate_tools_tokens(&tools);
        let budget = message_budget_for_tools(10_000, &tools);
        assert_eq!(budget, 10_000 - tools_t);
    }

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
                reasoning_original: None,
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
    fn structure_pr_get_diff_single_path_keeps_patch() {
        let raw = "Diff for o/r#1 path=src/lib.rs (80 bytes):\n\n\
diff --git a/src/lib.rs b/src/lib.rs\n\
--- a/src/lib.rs\n+++ b/src/lib.rs\n\
+fn main() {}\n";
        let structured = structure_tool_result("pr_get_diff", raw);
        assert!(structured.contains("path=src/lib.rs"));
        assert!(structured.contains("+fn main()"));
        assert!(!structured.contains("Per-file excerpts"));
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
            reasoning_original: None,
        };
        let llm = chat_message_to_llm(&msg);
        assert_eq!(llm.role, "user");
        assert!(llm.content.contains("missing required `repo`"));
    }

    #[test]
    fn trim_preserves_harness_nudge_content() {
        use crate::agent::tool_catalog::ToolCatalog;
        use crate::llm::LlmTurnMessage;
        let nudge = ToolCatalog::new().format_tool_args_nudge(
            "pr_get_overview",
            "pr_number",
            Some("19272"),
            Some("acme/widget"),
        );
        let mut messages = vec![
            LlmTurnMessage::new("system", "sys".repeat(5000)),
            LlmTurnMessage::new("user", "user".repeat(5000)),
            LlmTurnMessage::new("user", nudge.clone()),
        ];
        trim_llm_messages(&mut messages, 200, CompactionStrategy::Code);
        assert!(
            messages
                .iter()
                .any(|m| m.content.contains("Tool `pr_get_overview`")),
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
        assert!(
            tool_result_char_cap(CompactionStrategy::Ops, "pr_list_open")
                < tool_result_char_cap(CompactionStrategy::Ops, "pr_get_status")
        );
    }

    #[test]
    fn structure_pr_list_keeps_number_and_ci() {
        let raw = "open PR(s) in o/r (3):\n\
#19235  backport title  @alice  CI:failing  review:approved\n\
#19240  other  @bob  CI:passing  review:none";
        let structured = structure_tool_result("pr_list_open", raw);
        assert!(
            structured.contains("#19235 CI:failing"),
            "got: {structured}"
        );
        assert!(structured.contains("#19240 CI:passing"));
        assert!(!structured.contains("@alice"));
    }

    #[test]
    fn tool_results_are_not_focus_filtered() {
        let unrelated = prepare_tool_result_for_context("pr_get_status", "PR #9999 merged");
        assert!(unrelated.contains("#9999"));
        assert!(!unrelated.contains("omitted"));

        let related = prepare_tool_result_for_context("pr_get_overview", "PR #19235 overview");
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
    fn trim_preserves_reasoning_summary() {
        use crate::llm::LlmTurnMessage;
        let reasoning = LlmTurnMessage::new(
            "user",
            "[agent reasoning summary]\n- checked CI on #42\n- will call pr_get_diff",
        );
        let mut messages = vec![
            LlmTurnMessage::new("system", "sys".repeat(5000)),
            LlmTurnMessage::new("user", "user".repeat(5000)),
            reasoning,
        ];
        trim_llm_messages(&mut messages, 200, CompactionStrategy::Code);
        assert!(
            messages
                .iter()
                .any(|m| m.content.contains("[agent reasoning summary]")),
            "reasoning summary must survive trimming"
        );
    }

    #[test]
    fn strip_reasoning_summary_marker_for_context_panel() {
        let msg = LlmTurnMessage::new(
            "user",
            "[agent reasoning summary]\n\nThe user wants PR #42.",
        );
        let panel = format_llm_message_for_context_panel(&msg);
        assert_eq!(panel, "The user wants PR #42.");
        assert!(is_reasoning_summary_content(&msg.content));
    }

    #[test]
    fn is_reasoning_summary_content_detects_marker() {
        assert!(is_reasoning_summary_content(
            "[agent reasoning summary]\n- bullet"
        ));
        assert!(!is_reasoning_summary_content("tool_result(x):\nok"));
    }

    #[test]
    fn pack_session_history_includes_reasoning_messages() {
        use crate::store::{ChatMessage, ChatRole};
        let session_id = Uuid::new_v4();
        let history = vec![
            ChatMessage {
                id: Uuid::new_v4(),
                session_id,
                role: ChatRole::User,
                content: "analyze PR".into(),
                ts: Utc::now(),
                tool_name: None,
                tool_calls_json: None,
                reasoning_original: None,
            },
            ChatMessage {
                id: Uuid::new_v4(),
                session_id,
                role: ChatRole::Reasoning,
                content: "[agent reasoning summary]\n\nWill fetch PR overview.".into(),
                ts: Utc::now(),
                tool_name: None,
                tool_calls_json: None,
                reasoning_original: None,
            },
        ];
        let packed = pack_session_history(&history, 20, 64_000);
        assert_eq!(packed.len(), 2);
        assert!(packed[1].content.contains("[agent reasoning summary]"));
        assert!(packed[1].content.contains("Will fetch PR overview"));
    }

    #[test]
    fn analyze_context_trim_metadata_detects_markers() {
        let msgs = vec![
            LlmTurnMessage::new("system", "sys"),
            LlmTurnMessage::new(
                "user",
                "[12 earlier message(s) omitted from context — full transcript is in the session store]",
            ),
            LlmTurnMessage::new("user", "[session history summary]\n- asked about PR #42"),
            LlmTurnMessage::new("user", "[earlier context summary]\n- reran CI"),
            LlmTurnMessage::new(
                "user",
                "[summarized tool_result pr_get_diff]\n#42 title…",
            ),
        ];
        let (trimmed, note) = analyze_context_trim_metadata(&msgs);
        assert_eq!(trimmed, 12);
        assert_eq!(
            note.as_deref(),
            Some(
                "12 earlier turns omitted · session history summarized · earlier turns summarized · 1 tool output summarized"
            )
        );
    }

    #[test]
    fn analyze_context_trim_metadata_empty_when_clean() {
        let msgs = vec![
            LlmTurnMessage::new("system", "sys"),
            LlmTurnMessage::new("user", "hello"),
        ];
        let (trimmed, note) = analyze_context_trim_metadata(&msgs);
        assert_eq!(trimmed, 0);
        assert!(note.is_none());
    }

    #[test]
    fn is_rolling_summary_content_detects_prior_summaries() {
        assert!(is_rolling_summary_content(
            "[session history summary]\n- user asked about PR #42"
        ));
        assert!(is_rolling_summary_content(
            "[earlier context summary]\n- reran CI on acme/widget"
        ));
        assert!(!is_rolling_summary_content(
            "tool_result(pr_list_open):\n#1"
        ));
    }

    #[test]
    fn collapsible_indices_skip_harness_and_protect_tail() {
        let mut msgs = vec![LlmTurnMessage::new("system", "sys")];
        msgs.push(LlmTurnMessage::new("user", "msg1"));
        msgs.push(LlmTurnMessage::new(
            "user",
            "Identical `pr_list_open` with the same args",
        ));
        msgs.push(LlmTurnMessage::new("user", "msg2"));
        msgs.push(LlmTurnMessage::new("user", "tail1"));
        msgs.push(LlmTurnMessage::new("user", "tail2"));
        let idx = collapsible_indices_for_summary(&msgs, 2);
        assert_eq!(idx, vec![1, 3]);
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
        for _ in 0..16 {
            msgs.push(LlmTurnMessage::new(
                "user",
                format!("tool_result(pr_list_open):\n{}", "y".repeat(3000)),
            ));
        }
        let before = estimate_messages_tokens(&msgs);
        trim_llm_messages(&mut msgs, 6_500, CompactionStrategy::Ops);
        let after = estimate_messages_tokens(&msgs);
        assert_eq!(msgs[0].role, "system");
        assert!(msgs.len() >= 9);
        assert!(after < before);
        // Eight tail tool rows stay large by design; budget must allow that protected tail.
        assert!(after <= 7_200);
    }

    #[test]
    fn truncate_chars_respects_unicode_boundaries() {
        let msg = "你只需要给这些所有的 PR 的 changed files 总结一下，其他的不需要做。";
        let out = truncate_chars(msg, 20);
        assert!(out.ends_with('…'));
        assert!(std::str::from_utf8(out.as_bytes()).is_ok());
        assert!(out.chars().count() <= 21);
    }

    #[test]
    fn ops_compaction_preserves_ci_kind_and_verdict() {
        let body =
            "CI_KIND: actions_only\nverdict: flaky\nnoise line\n#19264 acme/widget CI failing";
        let out = summarize_ops_tool_content(
            "ci_analyze_pr_failures",
            &format!("tool_result(ci_analyze_pr_failures):\n{body}"),
        );
        assert!(out.contains("CI_KIND"));
        assert!(out.contains("verdict"));
    }

    #[test]
    fn compaction_strategy_variants_distinct() {
        assert_eq!(CompactionStrategy::Ops, CompactionStrategy::Ops);
        assert_ne!(CompactionStrategy::Ops, CompactionStrategy::Code);
    }

    #[test]
    fn coding_bash_compaction_keeps_exit_and_tail() {
        let body = (0..40)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let content = format!("exit: 1\n{body}");
        let out = summarize_coding_tool_content("bash_run", &content);
        assert!(out.contains("exit: 1"));
        assert!(out.contains("line 39"));
    }
}
