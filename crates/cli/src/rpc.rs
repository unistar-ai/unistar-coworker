use std::sync::Arc;

use serde::Deserialize;
use tokio::io::AsyncBufReadExt;

use coworker_core::app::{event_channel, AppState, SharedState};
use coworker_core::config::Config;
use coworker_core::engine::Engine;
use coworker_core::error::{CoworkerError, Result};

use super::chat::run_turn_with_progress;
use super::terminal::emit_json;

pub(crate) async fn run_rpc(
    config: Config,
    store: Arc<dyn coworker_core::store::Store>,
    session: Option<uuid::Uuid>,
    yes: bool,
    timeout: Option<u64>,
) -> Result<()> {
    if !config.chat.enabled {
        return Err(CoworkerError::Workflow(
            "chat disabled — set chat.enabled: true in coworker.yaml".into(),
        ));
    }
    let (tx, _rx) = event_channel();
    let event_tx = tx.clone();
    let state: SharedState = Arc::new(tokio::sync::RwLock::new(AppState::new(
        config.clone(),
        "rpc".into(),
    )));
    let engine = Arc::new(Engine::new(config, Arc::clone(&store), tx, Arc::clone(&state)).await);
    let mut rx = event_tx.subscribe();

    let progress_tx = event_tx.clone();
    tokio::spawn(async move {
        let mut prx = progress_tx.subscribe();
        while let Ok(ev) = prx.recv().await {
            if let coworker_core::app::AppEvent::ChatProgress(p) = ev {
                if let Some(line) = rpc_progress_json(&p) {
                    println!("{line}");
                }
            }
        }
    });

    let mut session_id = session;
    let mut lines = tokio::io::BufReader::new(tokio::io::stdin()).lines();
    while let Some(line) = lines
        .next_line()
        .await
        .map_err(|e| CoworkerError::Workflow(e.to_string()))?
    {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let req: RpcRequest = match serde_json::from_str(line) {
            Ok(r) => r,
            Err(e) => {
                emit_json(serde_json::json!({
                    "type": "error",
                    "code": "bad_request",
                    "error": e.to_string()
                }));
                continue;
            }
        };
        match req.op.as_str() {
            "chat" => {
                let msg = req.message.unwrap_or_default();
                if let Err(e) =
                    run_rpc_turn(&engine, &mut rx, &mut session_id, &msg, yes, timeout).await
                {
                    emit_json(serde_json::json!({
                        "type": "error",
                        "code": "turn_failed",
                        "error": e.to_string()
                    }));
                }
            }
            "get_state" => {
                let s = state.read().await;
                let snap = coworker_web::snapshot::build_snapshot_from(&s);
                emit_json(serde_json::json!({ "type": "state", "snapshot": snap }));
            }
            "cancel" => {
                engine.request_chat_cancel();
                emit_json(serde_json::json!({ "type": "cancelled" }));
            }
            "switch_profile" => match engine.switch_llm_profile(&req.profile).await {
                Ok(()) => emit_json(serde_json::json!({
                    "type": "profile",
                    "profile": req.profile
                })),
                Err(e) => emit_json(serde_json::json!({
                    "type": "error",
                    "code": "profile",
                    "error": e.to_string()
                })),
            },
            other => emit_json(serde_json::json!({
                "type": "error",
                "code": "unknown_op",
                "op": other
            })),
        }
    }
    Ok(())
}

async fn run_rpc_turn(
    engine: &Arc<Engine>,
    rx: &mut tokio::sync::broadcast::Receiver<coworker_core::app::AppEvent>,
    session_id: &mut Option<uuid::Uuid>,
    msg: &str,
    yes: bool,
    timeout: Option<u64>,
) -> Result<()> {
    let run_once = async {
        let (mut result, _streamed, mut pending) = run_turn_with_progress(
            engine,
            rx,
            true,
            None,
            false,
            engine.run_chat(*session_id, msg),
        )
        .await?;
        while result.awaiting_approval {
            let pa = match pending {
                Some(p) => p,
                None => break,
            };
            if !yes {
                emit_json(serde_json::json!({
                    "type": "error",
                    "code": "approval_required",
                    "session_id": result.session_id,
                    "pending_approval": {
                        "tool": pa.tool_name,
                        "args": coworker_core::agent::redact::redact_json_str(&pa.tool_args_json),
                        "description": pa.description,
                    }
                }));
                break;
            }
            let detail = engine
                .decide_approval(&pa.approval_id, true)
                .await
                .unwrap_or_default();
            let tool_args =
                serde_json::from_str(&pa.tool_args_json).unwrap_or_else(|_| serde_json::json!({}));
            let resume = coworker_core::agent::chat_loop::ResumeChatAfterApproval {
                approval_id: pa.approval_id,
                approved: true,
                detail,
                tool_name: pa.tool_name.clone(),
                tool_args,
            };
            let (r, _s, p) = run_turn_with_progress(
                engine,
                rx,
                true,
                None,
                false,
                engine.resume_chat_after_approval(pa.session_id, resume),
            )
            .await?;
            result = r;
            pending = p;
        }
        Ok::<_, CoworkerError>(result)
    };
    let result = match timeout {
        Some(secs) => {
            match tokio::time::timeout(std::time::Duration::from_secs(secs), run_once).await {
                Ok(r) => r?,
                Err(_) => {
                    emit_json(serde_json::json!({ "type": "error", "code": "timeout" }));
                    return Ok(());
                }
            }
        }
        None => run_once.await?,
    };
    emit_json(serde_json::json!({
        "type": "result",
        "ok": true,
        "session_id": result.session_id,
        "assistant": result.assistant_message,
        "tool_calls": result
            .tool_calls
            .iter()
            .map(|tc| serde_json::json!({ "tool": tc.tool_name, "output": tc.output }))
            .collect::<Vec<_>>(),
        "awaiting_approval": result.awaiting_approval,
    }));
    Ok(())
}

#[derive(Deserialize)]
struct RpcRequest {
    op: String,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    profile: String,
}

/// Map a streaming `ChatProgress` event to a single-line JSON progress record
/// for the RPC protocol (returns `None` for events with no RPC-relevant info).
fn rpc_progress_json(p: &coworker_core::agent::chat_loop::ChatProgress) -> Option<String> {
    use coworker_core::agent::chat_loop::ChatProgress;
    let v = match p {
        ChatProgress::TurnThinking { turn, elapsed_secs } => {
            serde_json::json!({"stage": "thinking", "turn": turn, "elapsed_secs": elapsed_secs})
        }
        ChatProgress::ToolStart { name, args_short } => {
            serde_json::json!({"stage": "tool_start", "name": name, "args": args_short})
        }
        ChatProgress::ToolDone {
            name,
            ok,
            elapsed_ms,
            ..
        } => {
            serde_json::json!({"stage": "tool_done", "name": name, "ok": ok, "elapsed_ms": elapsed_ms})
        }
        ChatProgress::AssistantPartial { text } => {
            serde_json::json!({"stage": "assistant", "text": text})
        }
        ChatProgress::ReasoningPartial { text } => {
            serde_json::json!({"stage": "reasoning", "text": text})
        }
        ChatProgress::ApprovalQueued {
            tool_name,
            description,
            ..
        } => {
            serde_json::json!({"stage": "approval", "tool": tool_name, "description": description})
        }
        ChatProgress::ApprovalResolved {
            tool_name,
            approved,
            ..
        } => {
            serde_json::json!({"stage": "approval_resolved", "tool": tool_name, "approved": approved})
        }
        ChatProgress::ReasoningSummary { preview, .. } => {
            serde_json::json!({"stage": "reasoning_summary", "preview": preview})
        }
        ChatProgress::ActivityFlow { text, .. } => {
            serde_json::json!({"stage": "activity", "text": text})
        }
        _ => return None,
    };
    Some(serde_json::to_string(&serde_json::json!({"type": "progress", "progress": v})).unwrap())
}
