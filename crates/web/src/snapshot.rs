use serde::Serialize;
use serde_json::{json, Value};

use coworker_core::agent::budget::TokenBudget;
use coworker_core::agent::context::truncate_chars;
use coworker_core::app::{AppState, Tab};

const WEB_CONTEXT_MSG_CHARS: usize = 4_000;
/// Tool output bodies in WS chat patches (full snapshot may still be large on first load).
const WEB_CHAT_PATCH_TOOL_OUTPUT_CHARS: usize = 8_000;

#[derive(Serialize)]
pub struct LlmProfileOption {
    pub id: String,
    pub model: String,
    pub base_url: String,
}

#[derive(Serialize)]
pub struct WebSnapshot {
    pub tab: String,
    pub tabs: Vec<String>,
    pub status: String,
    pub engine_busy: bool,
    pub engine_task_label: Option<String>,
    pub chat_enabled: bool,
    pub chat_busy: bool,
    pub chat_session_id: Option<String>,
    pub chat_lines: Vec<String>,
    /// Tool output bodies keyed by line index in `chat_lines` (expand in UI).
    pub chat_tool_outputs: std::collections::HashMap<String, String>,
    /// Raw (uncompressed) reasoning traces keyed by line index in `chat_lines`.
    /// Present only when LLM reasoning compression was applied for that line.
    pub chat_reasoning_originals: std::collections::HashMap<String, String>,
    /// Assistant message ids keyed by line index (branch regenerate).
    pub chat_assistant_ids: std::collections::HashMap<String, String>,
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
    pub approvals: Vec<Value>,
    pub log_filter: String,
    pub logs: Vec<Value>,
    pub config_path: String,
    pub llm_model: String,
    /// Active named preset from `llm_profiles`, if any.
    pub llm_profile: Option<String>,
    /// Selectable LLM presets (no secrets).
    pub llm_profile_options: Vec<LlmProfileOption>,
    pub github_ok: bool,
    pub llm_ok: bool,
    pub github_latency_ms: Option<u128>,
    pub llm_latency_ms: Option<u128>,
    pub mcp_servers: Vec<Value>,
    /// When true, mutating GitHub/MCP tools run without approval (`chat.auto_approve_mutations`).
    pub auto_approve_mutations: bool,
    /// Default Web UI theme from config (`dark` | `light`); user override in localStorage.
    pub ui_theme: String,
    pub app_version: String,
    pub upgrade_available: bool,
    pub latest_release: Option<String>,
    pub release_url: Option<String>,
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
    pub chat_session_id: Option<String>,
    pub chat_lines: Vec<String>,
    pub chat_tool_outputs: std::collections::HashMap<String, String>,
    pub chat_reasoning_originals: std::collections::HashMap<String, String>,
    pub chat_assistant_ids: std::collections::HashMap<String, String>,
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
        Tab::Approvals => "approvals",
        Tab::Logs => "logs",
        Tab::Config => "config",
    }
}

pub async fn build_snapshot(state: &coworker_core::app::SharedState) -> WebSnapshot {
    let s = state.read().await;
    build_snapshot_from(&s)
}

pub fn build_snapshot_from(s: &AppState) -> WebSnapshot {
    let tabs: Vec<String> = Tab::all_for_config(&s.config)
        .into_iter()
        .map(|t| tab_name(t).to_string())
        .collect();

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
    let live = web_live_transport_fields(s);

    WebSnapshot {
        tab: tab_name(s.tab).to_string(),
        tabs,
        status: s.status.clone(),
        engine_busy: s.engine_busy,
        engine_task_label: s.engine_task_label.clone(),
        chat_enabled: s.config.chat.enabled,
        chat_busy: s.chat_busy,
        chat_session_id: s.chat_session_id.map(|id| id.to_string()),
        chat_lines: s.chat_lines.clone(),
        chat_tool_outputs: s
            .chat_tool_outputs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect(),
        chat_reasoning_originals: s
            .chat_reasoning_originals
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect(),
        chat_assistant_ids: s
            .chat_assistant_ids
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        chat_history_revision: s.chat_history_revision,
        chat_context_revision: s.chat_context_revision,
        chat_streaming: live.chat_streaming,
        chat_reasoning: live.chat_reasoning,
        chat_tool_running: live.chat_tool_running,
        chat_tool_running_detail: live.chat_tool_running_detail,
        chat_tool_pending: live.chat_tool_pending,
        chat_turn_phase: live.chat_turn_phase,
        chat_reasoning_compressing: live.chat_reasoning_compressing,
        chat_activity_flow: live.chat_activity_flow,
        chat_context_visible: s.chat_context_visible,
        chat_context,
        chat_pending_approval,
        approval_dialog,
        approvals,
        log_filter: s.log_filter.label().to_string(),
        logs,
        config_path: s.config_path.clone(),
        llm_model: s.config.llm.model.clone(),
        llm_profile: s.config.llm_profile.clone(),
        llm_profile_options: build_llm_profile_options(s),
        github_ok: s.github_ok,
        llm_ok: s.llm_ok,
        github_latency_ms: s.github_latency_ms,
        llm_latency_ms: s.llm_latency_ms,
        mcp_servers: s
            .mcp_servers
            .iter()
            .map(|server| {
                json!({
                    "id": server.id,
                    "connected": server.connected,
                    "tool_count": server.tool_count,
                    "last_error": server.last_error,
                    "last_rpc_ms": server.last_rpc_ms,
                    "prefix": server.prefix,
                })
            })
            .collect(),
        auto_approve_mutations: s.config.chat.auto_approve_mutations,
        ui_theme: s.config.web_theme_id().to_string(),
        app_version: s.app_version.clone(),
        upgrade_available: s.upgrade_available,
        latest_release: s.latest_release.clone(),
        release_url: s.release_url.clone(),
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
                "description": sk.description,
                "always": sk.always,
                "skills": sk.skills,
                "tools": sk.tools,
                "argument_hint": sk.argument_hint,
                "intent_phrases": sk.intent_phrases,
                "intent_bonus_keywords": sk.intent_bonus_keywords,
            })).collect::<Vec<_>>(),
            "input_budget": c.input_budget,
            "context_limit": c.context_limit,
            "message_count": c.message_count,
            "messages": c.messages.iter().map(|m| {
                let mut row = json!({
                    "role": m.display_role,
                    "tokens": m.tokens,
                    "content": truncate_chars(&m.content, WEB_CONTEXT_MSG_CHARS),
                });
                if let Some(ref orig) = m.reasoning_original {
                    if let Some(obj) = row.as_object_mut() {
                        obj.insert(
                            "reasoning_original".into(),
                            json!(truncate_chars(orig, WEB_CONTEXT_MSG_CHARS)),
                        );
                    }
                }
                row
            }).collect::<Vec<_>>(),
            "runtime_context_revision": c.runtime_context_revision,
            "context_trimmed_turns": c.context_trimmed_turns,
            "context_summary_note": c.context_summary_note,
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
            "context_trimmed_turns": 0,
            "context_summary_note": Value::Null,
        })
    }
}

fn build_chat_pending_approval_json(s: &AppState) -> Option<Value> {
    s.chat_pending_approval.as_ref().map(|p| {
        json!({
            "id": p.id,
            "session_id": p.session_id,
            "tool_name": p.tool_name,
            "tool_args_json": coworker_core::agent::redact::redact_json_str(&p.tool_args_json),
        })
    })
}

fn build_approval_dialog_json(s: &AppState) -> Option<Value> {
    s.approval_dialog.as_ref().map(|d| {
        json!({
            "id": d.id,
            "tool_name": d.tool_name,
            "description": d.description,
            "tool_args_json": d.tool_args_json.as_deref().map(coworker_core::agent::redact::redact_json_str),
            "choice": format!("{:?}", d.choice),
            "deciding": d.deciding,
            "approve_armed": d.approve_armed(),
            "approve_arm_ms_remaining": d.approve_arm_ms_remaining(),
        })
    })
}

fn build_llm_profile_options(s: &AppState) -> Vec<LlmProfileOption> {
    let names = s.config.llm_profile_names();
    if names.is_empty() {
        return Vec::new();
    }
    names
        .into_iter()
        .filter_map(|id| {
            s.config.llm_profiles.get(&id).map(|cfg| LlmProfileOption {
                id,
                model: cfg.model.clone(),
                base_url: cfg.base_url.clone(),
            })
        })
        .collect()
}

struct WebLiveTransportFields {
    chat_streaming: Option<String>,
    chat_reasoning: Option<String>,
    chat_tool_running: Option<String>,
    chat_tool_running_detail: Option<String>,
    chat_tool_pending: Option<String>,
    chat_turn_phase: Option<String>,
    chat_reasoning_compressing: bool,
    chat_activity_flow: Option<Value>,
}

/// Live UI fields are only meaningful while `chat_busy`; strip them on the wire when idle
/// so stale streaming/reasoning cannot keep the live zone visible after a turn ends.
fn web_live_transport_fields(s: &AppState) -> WebLiveTransportFields {
    if s.chat_busy {
        WebLiveTransportFields {
            chat_streaming: s.chat_streaming.clone(),
            chat_reasoning: s.chat_reasoning.clone(),
            chat_tool_running: s.chat_tool_running.clone(),
            chat_tool_running_detail: s.chat_tool_running_detail.clone(),
            chat_tool_pending: s.chat_tool_pending.clone(),
            chat_turn_phase: s.chat_turn_phase().map(str::to_string),
            chat_reasoning_compressing: s.chat_reasoning_compressing,
            chat_activity_flow: build_chat_activity_flow_json(s),
        }
    } else {
        WebLiveTransportFields {
            chat_streaming: None,
            chat_reasoning: None,
            chat_tool_running: None,
            chat_tool_running_detail: None,
            chat_tool_pending: None,
            chat_turn_phase: None,
            chat_reasoning_compressing: false,
            chat_activity_flow: None,
        }
    }
}

pub async fn build_live_patch(state: &coworker_core::app::SharedState) -> WebLivePatch {
    let s = state.read().await;
    build_live_patch_from(&s)
}

pub fn build_live_patch_from(s: &AppState) -> WebLivePatch {
    let live = web_live_transport_fields(s);
    WebLivePatch {
        patch_type: "live",
        status: s.status.clone(),
        chat_busy: s.chat_busy,
        chat_streaming: live.chat_streaming,
        chat_reasoning: live.chat_reasoning,
        chat_tool_running: live.chat_tool_running,
        chat_tool_running_detail: live.chat_tool_running_detail,
        chat_tool_pending: live.chat_tool_pending,
        chat_turn_phase: live.chat_turn_phase,
        chat_reasoning_compressing: live.chat_reasoning_compressing,
        chat_activity_flow: live.chat_activity_flow,
    }
}

pub async fn build_chat_patch(state: &coworker_core::app::SharedState) -> WebChatPatch {
    let s = state.read().await;
    build_chat_patch_from(&s)
}

pub fn build_chat_patch_from(s: &AppState) -> WebChatPatch {
    let live = web_live_transport_fields(s);
    WebChatPatch {
        patch_type: "chat",
        status: s.status.clone(),
        chat_busy: s.chat_busy,
        chat_session_id: s.chat_session_id.map(|id| id.to_string()),
        chat_lines: s.chat_lines.clone(),
        chat_tool_outputs: s
            .chat_tool_outputs
            .iter()
            .map(|(k, v)| {
                (
                    k.to_string(),
                    truncate_chars(v, WEB_CHAT_PATCH_TOOL_OUTPUT_CHARS),
                )
            })
            .collect(),
        chat_reasoning_originals: s
            .chat_reasoning_originals
            .iter()
            .map(|(k, v)| {
                (
                    k.to_string(),
                    truncate_chars(v, WEB_CHAT_PATCH_TOOL_OUTPUT_CHARS),
                )
            })
            .collect(),
        chat_assistant_ids: s
            .chat_assistant_ids
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        chat_history_revision: s.chat_history_revision,
        chat_context_revision: s.chat_context_revision,
        chat_streaming: live.chat_streaming,
        chat_reasoning: live.chat_reasoning,
        chat_tool_running: live.chat_tool_running,
        chat_tool_running_detail: live.chat_tool_running_detail,
        chat_tool_pending: live.chat_tool_pending,
        chat_turn_phase: live.chat_turn_phase,
        chat_reasoning_compressing: live.chat_reasoning_compressing,
        chat_activity_flow: live.chat_activity_flow,
        chat_context_visible: s.chat_context_visible,
        chat_context: Some(build_chat_context_json(s)),
        chat_pending_approval: build_chat_pending_approval_json(s),
        approval_dialog: build_approval_dialog_json(s),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use coworker_core::app::AppState;
    use coworker_core::config::Config;
    use coworker_core::mcp::McpServerStatus;

    fn test_config_yaml(auto_approve: bool) -> Config {
        let yaml = format!(
            r#"
llm: {{ base_url: http://localhost:11434/v1, model: m, context_limit: 64000 }}
chat:
  enabled: true
  auto_approve_mutations: {auto_approve}
storage: {{ backend: json, path: ./data }}
"#
        );
        Config::load_from_str(&yaml).unwrap()
    }

    #[test]
    fn snapshot_exposes_auto_approve_mutations_from_config() {
        let snap = build_snapshot_from(&AppState::new(
            test_config_yaml(true),
            "coworker.yaml".into(),
        ));
        assert!(snap.auto_approve_mutations);

        let snap_off = build_snapshot_from(&AppState::new(
            test_config_yaml(false),
            "coworker.yaml".into(),
        ));
        assert!(!snap_off.auto_approve_mutations);
    }

    #[test]
    fn snapshot_exposes_upgrade_fields() {
        let mut app = AppState::new(test_config_yaml(false), "coworker.yaml".into());
        app.app_version = "2.1.0".into();
        app.upgrade_available = true;
        app.latest_release = Some("2.2.0".into());
        app.release_url = Some("https://example.com/release".into());
        let snap = build_snapshot_from(&app);
        assert_eq!(snap.app_version, "2.1.0");
        assert!(snap.upgrade_available);
        assert_eq!(snap.latest_release.as_deref(), Some("2.2.0"));
        assert_eq!(
            snap.release_url.as_deref(),
            Some("https://example.com/release")
        );
    }

    #[test]
    fn snapshot_exposes_mcp_server_metrics() {
        let mut app = AppState::new(test_config_yaml(false), "coworker.yaml".into());
        app.mcp_servers = vec![McpServerStatus {
            id: "slack".into(),
            connected: true,
            tool_count: 3,
            last_error: None,
            last_rpc_ms: Some(42),
            prefix: "slack_".into(),
        }];
        let snap = build_snapshot_from(&app);
        assert_eq!(snap.mcp_servers.len(), 1);
        let server = &snap.mcp_servers[0];
        assert_eq!(server["id"], "slack");
        assert_eq!(server["connected"], true);
        assert_eq!(server["tool_count"], 3);
        assert_eq!(server["last_rpc_ms"], 42);
        assert!(server["last_error"].is_null());
    }

    /// Keys that MUST appear in every `WebLivePatch` JSON. Adding or removing
    /// a field requires updating both the Rust struct and the React applicator in
    /// `web-ui/src/store/wsStore.ts` — this test forces that conscious update.
    const EXPECTED_LIVE_PATCH_KEYS: &[&str] = &[
        "_type",
        "status",
        "chat_busy",
        "chat_streaming",
        "chat_reasoning",
        "chat_tool_running",
        "chat_tool_running_detail",
        "chat_tool_pending",
        "chat_turn_phase",
        "chat_reasoning_compressing",
        "chat_activity_flow",
    ];

    /// Keys that MUST appear in every `WebChatPatch` JSON.
    const EXPECTED_CHAT_PATCH_KEYS: &[&str] = &[
        "_type",
        "status",
        "chat_busy",
        "chat_session_id",
        "chat_lines",
        "chat_tool_outputs",
        "chat_reasoning_originals",
        "chat_assistant_ids",
        "chat_history_revision",
        "chat_context_revision",
        "chat_streaming",
        "chat_reasoning",
        "chat_tool_running",
        "chat_tool_running_detail",
        "chat_tool_pending",
        "chat_turn_phase",
        "chat_reasoning_compressing",
        "chat_activity_flow",
        "chat_context_visible",
        "chat_context",
        "chat_pending_approval",
        "approval_dialog",
    ];

    fn obj_keys(v: &serde_json::Value) -> Vec<String> {
        let mut keys: Vec<String> = v
            .as_object()
            .expect("json object")
            .keys()
            .cloned()
            .collect();
        keys.sort();
        keys
    }

    fn expected_sorted(expected: &[&str]) -> Vec<String> {
        let mut v: Vec<String> = expected.iter().map(|s| s.to_string()).collect();
        v.sort();
        v
    }

    #[test]
    fn live_patch_serializes_expected_keys() {
        let app = AppState::new(test_config_yaml(false), "coworker.yaml".into());
        let patch = build_live_patch_from(&app);
        let v = serde_json::to_value(&patch).expect("serialize live patch");
        assert_eq!(obj_keys(&v), expected_sorted(EXPECTED_LIVE_PATCH_KEYS));
    }

    #[test]
    fn chat_patch_serializes_expected_keys() {
        let app = AppState::new(test_config_yaml(false), "coworker.yaml".into());
        let patch = build_chat_patch_from(&app);
        let v = serde_json::to_value(&patch).expect("serialize chat patch");
        assert_eq!(obj_keys(&v), expected_sorted(EXPECTED_CHAT_PATCH_KEYS));
    }

    #[test]
    fn live_patch_type_is_live() {
        let app = AppState::new(test_config_yaml(false), "coworker.yaml".into());
        let patch = build_live_patch_from(&app);
        let v = serde_json::to_value(&patch).expect("serialize");
        assert_eq!(v["_type"], "live");
    }

    #[test]
    fn chat_patch_type_is_chat() {
        let app = AppState::new(test_config_yaml(false), "coworker.yaml".into());
        let patch = build_chat_patch_from(&app);
        let v = serde_json::to_value(&patch).expect("serialize");
        assert_eq!(v["_type"], "chat");
    }

    #[test]
    fn chat_patch_truncates_tool_output_to_8k() {
        let mut app = AppState::new(test_config_yaml(false), "coworker.yaml".into());
        // Insert a tool output well beyond the 8k cap.
        let big = "x".repeat(WEB_CHAT_PATCH_TOOL_OUTPUT_CHARS * 2);
        app.chat_tool_outputs.insert(0, big.clone());
        let patch = build_chat_patch_from(&app);
        let v = serde_json::to_value(&patch).expect("serialize");
        let outputs = v["chat_tool_outputs"].as_object().expect("outputs map");
        // The key is the line index as a string ("0").
        let body = outputs
            .get("0")
            .and_then(|b| b.as_str())
            .expect("output body present");
        // `truncate_chars` appends an ellipsis (`…`) when truncating, so the
        // patched body is capped at `WEB_CHAT_PATCH_TOOL_OUTPUT_CHARS` chars
        // plus the ellipsis — never the full `big` body.
        assert!(
            body.chars().count() <= WEB_CHAT_PATCH_TOOL_OUTPUT_CHARS + 1,
            "chat patch must truncate tool output to <= {}+1 chars, got {}",
            WEB_CHAT_PATCH_TOOL_OUTPUT_CHARS,
            body.chars().count()
        );
        assert!(body.ends_with('…'));
        // Full snapshot does NOT truncate (separate contract).
        let snap = build_snapshot_from(&app);
        let snap_body = snap
            .chat_tool_outputs
            .get("0")
            .map(String::as_str)
            .expect("snapshot output present");
        assert_eq!(snap_body.len(), big.len());
    }

    #[test]
    fn live_patch_carries_streaming_and_phase_fields() {
        let mut app = AppState::new(test_config_yaml(false), "coworker.yaml".into());
        app.chat_busy = true;
        app.chat_streaming = Some("hello".into());
        app.chat_reasoning = Some("thinking…".into());
        app.chat_tool_running = Some("pr_get_diff".into());
        app.chat_reasoning_compressing = true;
        // `chat_turn_phase` is derived: tool_running wins → "tool".
        let patch = build_live_patch_from(&app);
        let v = serde_json::to_value(&patch).expect("serialize");
        assert_eq!(v["chat_busy"], true);
        assert_eq!(v["chat_streaming"], "hello");
        assert_eq!(v["chat_reasoning"], "thinking…");
        assert_eq!(v["chat_tool_running"], "pr_get_diff");
        assert_eq!(v["chat_turn_phase"], "tool");
        assert_eq!(v["chat_reasoning_compressing"], true);
    }

    #[test]
    fn live_patch_phase_is_none_when_not_busy() {
        let mut app = AppState::new(test_config_yaml(false), "coworker.yaml".into());
        // Stale in-memory live fields must not leak on the wire when idle.
        app.chat_streaming = Some("leftover".into());
        app.chat_reasoning = Some("leftover".into());
        app.chat_tool_running = Some("bash_run".into());
        let patch = build_live_patch_from(&app);
        let v = serde_json::to_value(&patch).expect("serialize");
        assert_eq!(v["chat_busy"], false);
        assert!(v["chat_turn_phase"].is_null());
        assert!(v["chat_streaming"].is_null());
        assert!(v["chat_reasoning"].is_null());
        assert!(v["chat_tool_running"].is_null());
    }

    /// Sanity check: full snapshot exposes the full set of UI-relevant keys.
    /// This guards against accidental field removal in `WebSnapshot`.
    #[test]
    fn snapshot_includes_core_keys() {
        let app = AppState::new(test_config_yaml(false), "coworker.yaml".into());
        let snap = build_snapshot_from(&app);
        let v = serde_json::to_value(&snap).expect("serialize snapshot");
        for key in [
            "tab",
            "tabs",
            "status",
            "engine_busy",
            "chat_enabled",
            "chat_busy",
            "chat_lines",
            "chat_tool_outputs",
            "chat_reasoning_originals",
            "chat_assistant_ids",
            "chat_history_revision",
            "chat_context_revision",
            "chat_streaming",
            "chat_reasoning",
            "chat_tool_running",
            "chat_tool_pending",
            "chat_turn_phase",
            "chat_reasoning_compressing",
            "chat_activity_flow",
            "chat_context_visible",
            "chat_context",
            "approval_dialog",
            "approvals",
            "logs",
            "mcp_servers",
            "llm_profile",
            "llm_profile_options",
            "auto_approve_mutations",
            "ui_theme",
        ] {
            assert!(v.get(key).is_some(), "snapshot missing key {key}");
        }
    }

    /// The context panel skill preview needs frontmatter metadata (description,
    /// always, skills, tools) in addition to the body. Verify these fields are
    /// serialized onto each skill_block.
    #[test]
    fn snapshot_skill_blocks_carry_frontmatter_metadata() {
        use coworker_core::agent::chat_loop::{ContextSkillBlock, ContextSnapshot};
        let mut app = AppState::new(test_config_yaml(false), "coworker.yaml".into());
        app.set_chat_context(ContextSnapshot {
            turn: 1,
            skill_blocks: vec![ContextSkillBlock {
                name: "my-prs".into(),
                body: "# My PRs\n...".into(),
                tokens: 140,
                description: "Author-focused open PR status".into(),
                always: false,
                skills: vec![],
                tools: vec!["pr_list_open".into()],
                argument_hint: "Author filter or repo".into(),
                intent_phrases: vec!["my pr".into(), "my open".into()],
                intent_bonus_keywords: vec!["@me".into()],
            }],
            ..Default::default()
        });
        let snap = build_snapshot_from(&app);
        let v = serde_json::to_value(&snap).expect("serialize snapshot");
        let blocks = &v["chat_context"]["skill_blocks"];
        assert_eq!(blocks[0]["name"], "my-prs");
        assert_eq!(blocks[0]["description"], "Author-focused open PR status");
        assert_eq!(blocks[0]["tools"][0], "pr_list_open");
        assert_eq!(blocks[0]["argument_hint"], "Author filter or repo");
        assert_eq!(blocks[0]["intent_phrases"][0], "my pr");
        assert_eq!(blocks[0]["intent_phrases"][1], "my open");
        assert_eq!(blocks[0]["intent_bonus_keywords"][0], "@me");
        assert!(blocks[0]["body"].as_str().unwrap().contains("My PRs"));
    }
}
