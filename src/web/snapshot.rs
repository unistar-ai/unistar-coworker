use chrono::NaiveDate;
use serde::Serialize;
use serde_json::{json, Value};

use crate::app::{AppState, Tab};
use crate::agent::budget::TokenBudget;
use crate::agent::context::truncate_chars;

const WEB_CONTEXT_MSG_CHARS: usize = 4_000;

#[derive(Serialize)]
pub struct WebSnapshot {
    pub tab: String,
    pub tabs: Vec<String>,
    pub status: String,
    pub engine_busy: bool,
    pub engine_workflow_id: Option<String>,
    pub chat_enabled: bool,
    pub chat_busy: bool,
    pub chat_lines: Vec<String>,
    /// Tool output bodies keyed by line index in `chat_lines` (expand in UI).
    pub chat_tool_outputs: std::collections::HashMap<String, String>,
    pub chat_history_revision: u64,
    pub chat_context_revision: u64,
    pub chat_streaming: Option<String>,
    pub chat_reasoning: Option<String>,
    pub chat_tool_running: Option<String>,
    pub chat_tool_running_detail: Option<String>,
    pub chat_tool_pending: Option<String>,
    pub chat_turn_phase: Option<String>,
    pub chat_reasoning_compressing: bool,
    pub chat_activity_flow: Option<Value>,
    pub chat_context_visible: bool,
    pub chat_context: Option<Value>,
    pub chat_pending_approval: Option<Value>,
    pub approval_dialog: Option<Value>,
    pub digest_history: Vec<Value>,
    pub digest_bodies: std::collections::HashMap<String, String>,
    pub selected_digest_date: Option<String>,
    pub prs: Vec<Value>,
    pub pr_filter: String,
    pub pr_sort: String,
    pub selected_pr_index: usize,
    pub pr_overview: Option<String>,
    pub pr_overview_loading: bool,
    pub approvals: Vec<Value>,
    pub log_filter: String,
    pub logs: Vec<Value>,
    pub config_path: String,
    pub repos: Vec<String>,
    pub llm_model: String,
    pub github_ok: bool,
    pub llm_ok: bool,
    pub github_latency_ms: Option<u128>,
    pub llm_latency_ms: Option<u128>,
    pub attach_mode: bool,
}

/// Lightweight WS patch for streaming / tool progress (avoids full snapshot flood).
#[derive(Serialize)]
pub struct WebLivePatch {
    #[serde(rename = "_type")]
    pub patch_type: &'static str,
    pub status: String,
    pub chat_busy: bool,
    pub chat_streaming: Option<String>,
    pub chat_reasoning: Option<String>,
    pub chat_tool_running: Option<String>,
    pub chat_tool_running_detail: Option<String>,
    pub chat_tool_pending: Option<String>,
    pub chat_turn_phase: Option<String>,
    pub chat_reasoning_compressing: bool,
    pub chat_activity_flow: Option<Value>,
}

/// Chat-pane WS patch (history, context, approvals) without digest/PR/log payload.
#[derive(Serialize)]
pub struct WebChatPatch {
    #[serde(rename = "_type")]
    pub patch_type: &'static str,
    pub status: String,
    pub chat_busy: bool,
    pub chat_lines: Vec<String>,
    pub chat_tool_outputs: std::collections::HashMap<String, String>,
    pub chat_history_revision: u64,
    pub chat_context_revision: u64,
    pub chat_streaming: Option<String>,
    pub chat_reasoning: Option<String>,
    pub chat_tool_running: Option<String>,
    pub chat_tool_running_detail: Option<String>,
    pub chat_tool_pending: Option<String>,
    pub chat_turn_phase: Option<String>,
    pub chat_reasoning_compressing: bool,
    pub chat_activity_flow: Option<Value>,
    pub chat_context_visible: bool,
    pub chat_context: Option<Value>,
    pub chat_pending_approval: Option<Value>,
    pub approval_dialog: Option<Value>,
}

fn tab_name(tab: Tab) -> &'static str {
    match tab {
        Tab::Chat => "chat",
        Tab::Dashboard => "dashboard",
        Tab::Prs => "prs",
        Tab::Approvals => "approvals",
        Tab::Logs => "logs",
        Tab::Config => "config",
    }
}

fn date_key(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

pub async fn build_snapshot(state: &crate::app::SharedState) -> WebSnapshot {
    let s = state.read().await;
    build_snapshot_from(&s)
}

pub fn build_snapshot_from(s: &AppState) -> WebSnapshot {
    let tabs: Vec<String> = Tab::all_for_config(&s.config)
        .into_iter()
        .map(|t| tab_name(t).to_string())
        .collect();

    let digest_history: Vec<Value> = s
        .digest_history
        .iter()
        .map(|m| {
            json!({
                "date": m.date.format("%Y-%m-%d").to_string(),
                "complete": m.summary.complete,
                "needs_attention": m.summary.needs_attention,
                "ignorable": m.summary.ignorable,
                "flaky_candidates": m.summary.flaky_candidates,
                "policy_gates": m.summary.policy_gates,
                "duration_label": m.summary.duration_label(),
            })
        })
        .collect();

    let digest_bodies: std::collections::HashMap<String, String> = s
        .digest_bodies
        .iter()
        .map(|(d, body)| (date_key(*d), body.clone()))
        .collect();

    let selected_digest_date = s
        .digest_history
        .get(s.selected_index)
        .map(|m| date_key(m.date));

    let prs: Vec<Value> = s
        .sorted_filtered_prs()
        .into_iter()
        .map(|p| {
            json!({
                "repo": p.repo,
                "number": p.number,
                "title": p.title,
                "author": p.author,
                "fetched_at": p.fetched_at.to_rfc3339(),
                "ci_summary": p.ci_summary,
                "review_summary": p.review_summary,
                "triage_note": p.triage_note,
                "is_draft": p.is_draft,
            })
        })
        .collect();

    let pr_overview = s.selected_pr_overview().map(str::to_string);
    let pr_overview_loading = s.selected_pr_overview_loading();

    let approvals: Vec<Value> = s
        .approvals
        .iter()
        .map(|a| {
            json!({
                "id": a.id,
                "kind": format!("{:?}", a.kind),
                "description": a.description,
                "created_at": a.created_at.to_rfc3339(),
                "repo": a.repo,
                "pr_number": a.pr_number,
                "run_id": a.run_id,
                "target_branch": a.target_branch,
                "status": format!("{:?}", a.status),
                "comment_body": a.comment_body,
                "issue_number": a.issue_number,
                "label": a.label,
            })
        })
        .collect();

    let logs: Vec<Value> = s
        .filtered_logs()
        .into_iter()
        .rev()
        .take(200)
        .map(|l| {
            json!({
                "level": l.level,
                "message": l.message,
                "ts": l.ts.to_rfc3339(),
            })
        })
        .collect();

    let chat_context = Some(build_chat_context_json(s));
    let chat_pending_approval = build_chat_pending_approval_json(s);
    let approval_dialog = build_approval_dialog_json(s);

    WebSnapshot {
        tab: tab_name(s.tab).to_string(),
        tabs,
        status: s.status.clone(),
        engine_busy: s.engine_busy,
        engine_workflow_id: s.engine_workflow_id.clone(),
        chat_enabled: s.config.chat.enabled,
        chat_busy: s.chat_busy,
        chat_lines: s.chat_lines.clone(),
        chat_tool_outputs: s
            .chat_tool_outputs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect(),
        chat_history_revision: s.chat_history_revision,
        chat_context_revision: s.chat_context_revision,
        chat_streaming: s.chat_streaming.clone(),
        chat_reasoning: s.chat_reasoning.clone(),
        chat_tool_running: s.chat_tool_running.clone(),
        chat_tool_running_detail: s.chat_tool_running_detail.clone(),
        chat_tool_pending: s.chat_tool_pending.clone(),
        chat_turn_phase: s.chat_turn_phase().map(str::to_string),
        chat_reasoning_compressing: s.chat_reasoning_compressing,
        chat_activity_flow: build_chat_activity_flow_json(s),
        chat_context_visible: s.chat_context_visible,
        chat_context,
        chat_pending_approval,
        approval_dialog,
        digest_history,
        digest_bodies,
        selected_digest_date,
        prs,
        pr_filter: s.pr_filter.label().to_string(),
        pr_sort: s.pr_sort.label().to_string(),
        selected_pr_index: s.selected_index,
        pr_overview,
        pr_overview_loading,
        approvals,
        log_filter: s.log_filter.label().to_string(),
        logs,
        config_path: s.config_path.clone(),
        repos: s.config.repos.clone(),
        llm_model: s.config.llm.model.clone(),
        github_ok: s.github_ok,
        llm_ok: s.llm_ok,
        github_latency_ms: s.github_latency_ms,
        llm_latency_ms: s.llm_latency_ms,
        attach_mode: s.attach_mode,
    }
}

fn build_chat_activity_flow_json(s: &AppState) -> Option<Value> {
    s.chat_activity_flow.as_ref().map(|f| {
        json!({
            "kind": format!("{:?}", f.kind),
            "text": f.text,
        })
    })
}

fn build_chat_context_json(s: &AppState) -> Value {
    if let Some(c) = s.chat_context.as_ref() {
        json!({
            "turn": c.turn,
            "message_tokens": c.message_tokens,
            "tools_tokens": c.tools_tokens,
            "tools_body": c.tools_body,
            "tool_names": c.tool_names,
            "skills_tokens": c.skills_tokens,
            "skill_blocks": c.skill_blocks.iter().map(|sk| json!({
                "name": sk.name,
                "tokens": sk.tokens,
                "body": sk.body,
            })).collect::<Vec<_>>(),
            "input_budget": c.input_budget,
            "context_limit": c.context_limit,
            "message_count": c.message_count,
            "messages": c.messages.iter().map(|m| json!({
                "role": m.display_role,
                "tokens": m.tokens,
                "content": truncate_chars(&m.content, WEB_CONTEXT_MSG_CHARS),
            })).collect::<Vec<_>>(),
            "runtime_context_revision": c.runtime_context_revision,
        })
    } else {
        let budget = TokenBudget::from_config(s.config.llm.context_limit);
        json!({
            "turn": 0,
            "message_tokens": 0,
            "tools_tokens": 0,
            "tools_body": "",
            "tool_names": [],
            "skills_tokens": 0,
            "skill_blocks": [],
            "input_budget": budget.input_budget(),
            "context_limit": budget.context_limit,
            "message_count": 0,
            "messages": [],
            "runtime_context_revision": Value::Null,
        })
    }
}

fn build_chat_pending_approval_json(s: &AppState) -> Option<Value> {
    s.chat_pending_approval.as_ref().map(|p| {
        json!({
            "id": p.id,
            "session_id": p.session_id,
            "tool_name": p.tool_name,
            "tool_args_json": p.tool_args_json,
        })
    })
}

fn build_approval_dialog_json(s: &AppState) -> Option<Value> {
    s.approval_dialog.as_ref().map(|d| {
        json!({
            "id": d.id,
            "tool_name": d.tool_name,
            "description": d.description,
            "choice": format!("{:?}", d.choice),
            "deciding": d.deciding,
            "approve_armed": d.approve_armed(),
            "approve_arm_ms_remaining": d.approve_arm_ms_remaining(),
        })
    })
}

pub async fn build_live_patch(state: &crate::app::SharedState) -> WebLivePatch {
    let s = state.read().await;
    build_live_patch_from(&s)
}

pub fn build_live_patch_from(s: &AppState) -> WebLivePatch {
    WebLivePatch {
        patch_type: "live",
        status: s.status.clone(),
        chat_busy: s.chat_busy,
        chat_streaming: s.chat_streaming.clone(),
        chat_reasoning: s.chat_reasoning.clone(),
        chat_tool_running: s.chat_tool_running.clone(),
        chat_tool_running_detail: s.chat_tool_running_detail.clone(),
        chat_tool_pending: s.chat_tool_pending.clone(),
        chat_turn_phase: s.chat_turn_phase().map(str::to_string),
        chat_reasoning_compressing: s.chat_reasoning_compressing,
        chat_activity_flow: build_chat_activity_flow_json(s),
    }
}

pub async fn build_chat_patch(state: &crate::app::SharedState) -> WebChatPatch {
    let s = state.read().await;
    build_chat_patch_from(&s)
}

pub fn build_chat_patch_from(s: &AppState) -> WebChatPatch {
    WebChatPatch {
        patch_type: "chat",
        status: s.status.clone(),
        chat_busy: s.chat_busy,
        chat_lines: s.chat_lines.clone(),
        chat_tool_outputs: s
            .chat_tool_outputs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect(),
        chat_history_revision: s.chat_history_revision,
        chat_context_revision: s.chat_context_revision,
        chat_streaming: s.chat_streaming.clone(),
        chat_reasoning: s.chat_reasoning.clone(),
        chat_tool_running: s.chat_tool_running.clone(),
        chat_tool_running_detail: s.chat_tool_running_detail.clone(),
        chat_tool_pending: s.chat_tool_pending.clone(),
        chat_turn_phase: s.chat_turn_phase().map(str::to_string),
        chat_reasoning_compressing: s.chat_reasoning_compressing,
        chat_activity_flow: build_chat_activity_flow_json(s),
        chat_context_visible: s.chat_context_visible,
        chat_context: Some(build_chat_context_json(s)),
        chat_pending_approval: build_chat_pending_approval_json(s),
        approval_dialog: build_approval_dialog_json(s),
    }
}
