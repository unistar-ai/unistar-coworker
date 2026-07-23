//! Approval decision flow (shared by TUI modal and WebUI).

use std::sync::Arc;

use uuid::Uuid;

use super::{SharedState, APPROVAL_ARM_DELAY};
use crate::engine::Engine;

pub async fn spawn_approval_decision(
    state: &SharedState,
    engine: &Arc<Engine>,
    id: Uuid,
    approve: bool,
    decision_reason: Option<String>,
) {
    if approve {
        let armed = {
            let s = state.read().await;
            match &s.approval_dialog {
                Some(d) if d.id == id => d.approve_armed(),
                _ => true,
            }
        };
        if !armed {
            let mut s = state.write().await;
            s.status = format!(
                "approval: wait {}ms before approve",
                APPROVAL_ARM_DELAY.as_millis()
            );
            return;
        }
    }

    {
        let mut s = state.write().await;
        if !s.try_begin_approval_decision(id, approve) {
            return;
        }
    }

    let engine = Arc::clone(engine);
    let state = state.clone();
    tokio::spawn(async move {
        let resume_ctx = {
            let s = state.read().await;
            s.chat_pending_approval
                .as_ref()
                .filter(|p| p.id == id)
                .cloned()
        };

        let result = engine
            .decide_approval(&id, approve, decision_reason.as_deref())
            .await;
        let mut s = state.write().await;
        s.finish_approval_decision(id);
        s.close_approval_dialog();
        match result {
            Ok(msg) => {
                s.resolve_chat_approval(id, approve, &msg);
                if approve {
                    s.push_log("info", format!("approved: {msg}"));
                    s.status = msg.clone();
                } else {
                    s.push_log("info", format!("denied: {msg}"));
                    s.status = "approval denied".into();
                }

                if let Some(pending) = resume_ctx {
                    let tool_args = serde_json::from_str(&pending.tool_args_json)
                        .unwrap_or_else(|_| serde_json::json!({}));
                    let resume = crate::agent::chat_loop::ResumeChatAfterApproval {
                        approval_id: id,
                        approved: approve,
                        detail: msg,
                        tool_name: pending.tool_name,
                        tool_args,
                        tool_call_id: pending.tool_call_id,
                    };
                    let session_id = pending.session_id;
                    drop(s);
                    if let Err(e) = engine.resume_chat_after_approval(session_id, resume).await {
                        let mut s = state.write().await;
                        s.push_log("error", format!("chat resume after approval failed: {e}"));
                        s.status = format!("approval completed, but chat resume failed: {e}");
                    }
                }
            }
            Err(e) => {
                let detail = format!("approval failed: {e}");
                s.resolve_chat_approval(id, false, &detail);
                s.push_log("error", detail.clone());
                s.status = detail.clone();

                if let Some(pending) = resume_ctx {
                    let tool_args = serde_json::from_str(&pending.tool_args_json)
                        .unwrap_or_else(|_| serde_json::json!({}));
                    let resume = crate::agent::chat_loop::ResumeChatAfterApproval {
                        approval_id: id,
                        approved: false,
                        detail: detail.clone(),
                        tool_name: pending.tool_name,
                        tool_args,
                        tool_call_id: pending.tool_call_id,
                    };
                    let session_id = pending.session_id;
                    drop(s);
                    if let Err(e) = engine.resume_chat_after_approval(session_id, resume).await {
                        let mut s = state.write().await;
                        s.push_log("error", format!("chat resume after approval failed: {e}"));
                        s.status = format!("approval failed; chat resume also failed: {e}");
                    }
                }
            }
        }
    });
}
