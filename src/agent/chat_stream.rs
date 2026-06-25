use chrono::Utc;
use uuid::Uuid;

use crate::error::Result;
use crate::llm::chat::ChatAgentStep;
use crate::llm::ChatAgentAction;
use crate::store::{ChatMessage, ChatRole, Store};

pub(crate) fn interim_assistant_message(step: &ChatAgentStep) -> Option<String> {
    if step.action != ChatAgentAction::Tool {
        return None;
    }
    let message = step.message.trim();
    if message.is_empty()
        || message.starts_with('{')
        || crate::agent::context::is_tool_result_transcript(message)
    {
        return None;
    }
    if message.len() > 800 {
        return None;
    }
    Some(message.to_string())
}

pub(crate) async fn persist_interim_assistant_message(
    store: &dyn Store,
    session_id: &Uuid,
    step: &ChatAgentStep,
) -> Result<()> {
    let Some(message) = interim_assistant_message(step) else {
        return Ok(());
    };
    store
        .append_chat_message(&ChatMessage {
            id: Uuid::new_v4(),
            session_id: *session_id,
            role: ChatRole::Assistant,
            content: message,
            ts: Utc::now(),
            tool_name: None,
            tool_calls_json: None,
        })
        .await
}
