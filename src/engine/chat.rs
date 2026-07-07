use std::sync::Arc;

use uuid::Uuid;

use crate::agent::chat_loop::{
    is_chat_cancelled, resume_chat_after_approval, run_chat_turn, ChatTurnInput, ChatTurnResult,
    ResumeChatAfterApproval,
};
use crate::app::load_chat_session_ui;
use crate::error::Result;

use super::Engine;

impl Engine {
    pub async fn run_chat(
        &self,
        session_id: Option<Uuid>,
        user_message: &str,
    ) -> Result<ChatTurnResult> {
        {
            let mut s = self.state.write().await;
            s.chat_busy = true;
            s.chat_scroll_from_bottom = 0;
            s.push_chat_line(format!("you> {user_message}"));
            s.push_log("info", format!("chat: {}", truncate_log(user_message)));
        }

        self.reset_chat_cancel();
        let progress = Some(self.events.clone());
        let cancel = self.chat_cancel_flag();
        let config = self.config.read().expect("config lock").clone();
        let result = run_chat_turn(
            &config,
            Arc::clone(&self.store),
            Arc::clone(&self.github),
            Arc::clone(&self.mcp),
            Arc::clone(&self.llm),
            ChatTurnInput {
                session_id,
                user_message: user_message.to_string(),
                progress,
                cancel: Some(cancel),
                resume: None,
            },
        )
        .await;

        self.apply_chat_turn_result(&result).await;
        result
    }

    pub async fn resume_chat_after_approval(
        &self,
        session_id: uuid::Uuid,
        resume: ResumeChatAfterApproval,
    ) -> Result<ChatTurnResult> {
        {
            let mut s = self.state.write().await;
            s.chat_busy = true;
            s.push_log(
                "info",
                format!(
                    "chat: resuming after approval {} ({})",
                    resume.approval_id,
                    if resume.approved {
                        "approved"
                    } else {
                        "denied"
                    }
                ),
            );
            s.status = "chat: resuming after approval…".into();
        }

        self.reset_chat_cancel();
        let progress = Some(self.events.clone());
        let cancel = self.chat_cancel_flag();
        let config = self.config.read().expect("config lock").clone();
        let result = resume_chat_after_approval(
            &config,
            Arc::clone(&self.store),
            Arc::clone(&self.github),
            Arc::clone(&self.mcp),
            Arc::clone(&self.llm),
            session_id,
            resume,
            progress,
            Some(cancel),
        )
        .await;

        self.apply_chat_turn_result(&result).await;
        result
    }

    /// Shared post-turn state update for [`run_chat`] and [`resume_chat_after_approval`]:
    /// refreshes the store, clears busy/tool/reasoning flags, reloads chat session UI,
    /// sets status text, and broadcasts `ChatReply` on success.
    ///
    /// Store refresh errors are logged but no longer propagated, matching the
    /// turn result taking precedence (the original code propagated refresh
    /// errors and would shadow the chat result).
    async fn apply_chat_turn_result(&self, result: &Result<ChatTurnResult>) {
        if let Err(e) = self.refresh_store().await {
            tracing::warn!("chat: refresh_store failed: {e}");
        }
        {
            let mut s = self.state.write().await;
            s.chat_busy = false;
            s.set_chat_tool_pending(None);
            s.set_chat_tool_running(None);
            s.set_chat_reasoning(None);
            s.set_chat_activity_flow(None);
            s.set_chat_reasoning_compressing(false);
            match result {
                Ok(r) if r.awaiting_approval => {
                    s.chat_session_id = Some(r.session_id);
                    s.set_chat_streaming(None);
                    if let Err(e) =
                        load_chat_session_ui(&mut s, self.store.as_ref(), r.session_id).await
                    {
                        s.push_log("warn", format!("chat reload failed: {e}"));
                    }
                    s.status = "awaiting approval — confirm in popup".into();
                }
                Ok(r) => {
                    s.chat_session_id = Some(r.session_id);
                    s.set_chat_streaming(None);
                    if let Err(e) =
                        load_chat_session_ui(&mut s, self.store.as_ref(), r.session_id).await
                    {
                        if !r.assistant_message.is_empty()
                            && !crate::agent::context::is_tool_result_transcript(
                                &r.assistant_message,
                            )
                        {
                            s.push_chat_line(format!("assistant> {}", r.assistant_message));
                        }
                        s.push_log("warn", format!("chat reload failed: {e}"));
                    }
                    s.status = "chat ready".into();
                }
                Err(e) if is_chat_cancelled(e) => {
                    s.set_chat_streaming(None);
                    s.push_chat_line("chat> cancelled".to_string());
                    s.status = "chat ready".into();
                }
                Err(e) => {
                    s.set_chat_streaming(None);
                    s.push_chat_line(format!("error> {e}"));
                    s.status = format!("chat error: {e}");
                }
            }
        }

        if result.is_ok() {
            let _ = self.events.send(crate::app::AppEvent::ChatReply);
        }
    }
}

fn truncate_log(text: &str) -> String {
    crate::agent::context::truncate_chars(text, 80)
}
