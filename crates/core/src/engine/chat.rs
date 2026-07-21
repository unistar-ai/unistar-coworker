use std::sync::Arc;

use uuid::Uuid;

use crate::agent::ask_user_tool::question_id_from_pending_transcript;
use crate::agent::chat_loop::{
    is_chat_cancelled, resume_chat_after_approval, resume_chat_after_user_answer, run_chat_turn,
    ChatTurnInput, ChatTurnResult, ResumeChatAfterApproval, ResumeChatAfterUserAnswer,
};
use crate::agent::context::is_tool_user_question_pending_transcript;
use crate::app::load_chat_session_ui;
use crate::error::Result;
use crate::llm::chat::LlmToolCall;
use crate::store::{ChatMessage, ChatRole, Store};

use super::Engine;

impl Engine {
    pub async fn run_chat(
        &self,
        session_id: Option<Uuid>,
        user_message: &str,
    ) -> Result<ChatTurnResult> {
        // If the session is paused on `ask_user`, the next user message is the answer.
        if let Some(sid) = session_id {
            if let Some(resume) =
                pending_ask_user_resume(self.store.as_ref(), sid, user_message).await?
            {
                {
                    let mut s = self.state.write().await;
                    s.chat_busy = true;
                    s.chat_scroll_from_bottom = 0;
                    s.push_chat_line(format!("you> {user_message}"));
                    s.push_log(
                        "info",
                        format!("chat: answering ask_user {}", resume.question_id),
                    );
                    s.status = "chat: resuming after user answer…".into();
                }
                self.reset_chat_cancel();
                let progress = Some(self.events.clone());
                let cancel = self.chat_cancel_flag();
                let config = self.config.read().expect("config lock").clone();
                let result = resume_chat_after_user_answer(
                    &config,
                    Arc::clone(&self.store),
                    Arc::clone(&self.github),
                    Arc::clone(&self.mcp),
                    Arc::clone(&self.llm),
                    sid,
                    resume,
                    progress,
                    Some(cancel),
                )
                .await;
                self.apply_chat_turn_result(&result).await;
                return result;
            }
        }

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
                resume_user_answer: None,
                regenerate_from: None,
            },
        )
        .await;

        self.apply_chat_turn_result(&result).await;
        result
    }

    pub async fn regenerate_chat(
        &self,
        session_id: Uuid,
        assistant_message_id: Uuid,
    ) -> Result<ChatTurnResult> {
        {
            let mut s = self.state.write().await;
            s.chat_busy = true;
            s.chat_scroll_from_bottom = 0;
            s.push_log(
                "info",
                format!("chat: regenerating message {assistant_message_id}"),
            );
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
                session_id: Some(session_id),
                user_message: String::new(),
                progress,
                cancel: Some(cancel),
                resume: None,
                resume_user_answer: None,
                regenerate_from: Some(assistant_message_id),
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

    pub async fn resume_chat_after_user_answer(
        &self,
        session_id: uuid::Uuid,
        resume: ResumeChatAfterUserAnswer,
    ) -> Result<ChatTurnResult> {
        {
            let mut s = self.state.write().await;
            s.chat_busy = true;
            s.push_log(
                "info",
                format!("chat: resuming after ask_user {}", resume.question_id),
            );
            s.status = "chat: resuming after user answer…".into();
        }

        self.reset_chat_cancel();
        let progress = Some(self.events.clone());
        let cancel = self.chat_cancel_flag();
        let config = self.config.read().expect("config lock").clone();
        let result = resume_chat_after_user_answer(
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

    /// Shared post-turn state update for chat / resume entry points:
    /// refreshes the store, clears busy/tool/reasoning flags, reloads chat session UI,
    /// sets status text, and broadcasts `ChatReply` on success.
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
                Ok(r) if r.awaiting_user_input => {
                    s.chat_session_id = Some(r.session_id);
                    s.set_chat_streaming(None);
                    if let Err(e) =
                        load_chat_session_ui(&mut s, self.store.as_ref(), r.session_id).await
                    {
                        s.push_log("warn", format!("chat reload failed: {e}"));
                    }
                    s.status = "awaiting your answer".into();
                }
                Ok(r) => {
                    s.chat_session_id = Some(r.session_id);
                    s.set_chat_streaming(None);
                    s.set_chat_pending_user_question(None);
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

/// Build a resume payload when the latest tool transcript is still awaiting an answer.
async fn pending_ask_user_resume(
    store: &dyn Store,
    session_id: Uuid,
    answer: &str,
) -> Result<Option<ResumeChatAfterUserAnswer>> {
    let history = store.list_chat_messages(&session_id, 10_000).await?;
    let Some(pending) = history
        .iter()
        .rev()
        .find(|m| m.role == ChatRole::Tool && is_tool_user_question_pending_transcript(&m.content))
    else {
        return Ok(None);
    };
    // Only resume if nothing newer than the pending question (no later user/assistant).
    if history
        .iter()
        .rev()
        .take_while(|m| m.id != pending.id)
        .any(|m| matches!(m.role, ChatRole::User | ChatRole::Assistant))
    {
        return Ok(None);
    }
    let Some(question_id) = question_id_from_pending_transcript(&pending.content) else {
        return Ok(None);
    };
    let tool_args = pending
        .tool_calls_json
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    let tool_call_id = recover_ask_user_tool_call_id(&history, pending).unwrap_or_default();
    Ok(Some(ResumeChatAfterUserAnswer {
        question_id,
        answer: answer.to_string(),
        tool_call_id,
        tool_args,
    }))
}

fn recover_ask_user_tool_call_id(history: &[ChatMessage], pending: &ChatMessage) -> Option<String> {
    history
        .iter()
        .rev()
        .skip_while(|m| m.id != pending.id)
        .skip(1)
        .find(|m| m.role == ChatRole::Assistant && m.tool_calls_json.is_some())
        .and_then(|m| {
            let calls: Vec<LlmToolCall> = serde_json::from_str(m.tool_calls_json.as_ref()?).ok()?;
            calls
                .into_iter()
                .find(|c| c.name == "ask_user")
                .map(|c| c.id)
        })
}
