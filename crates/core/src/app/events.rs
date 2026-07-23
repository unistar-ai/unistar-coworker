//! Engine → UI state updates (shared by TUI and WebUI).

use super::{AppEvent, ChatPendingApproval, ChatPendingUserQuestion, SharedState};
use crate::agent::chat_loop::{ChatActivityFlow, ChatProgress};

pub async fn apply_event(state: &SharedState, ev: AppEvent) {
    let mut s = state.write().await;
    match ev {
        AppEvent::StoreUpdated => {
            let prev = s.last_pending_approval_count;
            s.maybe_notify_new_approvals(prev);
            s.status = "store updated".into();
        }
        AppEvent::LogLine(l) => s.push_log(&l.level, l.message),
        AppEvent::BackgroundTaskStarted { label } => {
            s.engine_busy = true;
            s.engine_task_label = Some(label.clone());
            s.status = format!("running {label}");
        }
        AppEvent::BackgroundTaskFinished { label, ok, message } => {
            s.engine_busy = false;
            s.engine_task_label = None;
            s.status = if ok {
                message.clone()
            } else {
                format!("error: {message}")
            };
            s.push_log("info", format!("{label} finished: {message}"));
        }
        AppEvent::StatusMessage(m) => {
            s.status = m.clone();
            s.push_log("info", m);
        }
        AppEvent::ChatProgress(p) => {
            match &p {
                ChatProgress::TurnThinking { .. } => {
                    s.set_chat_streaming(None);
                    s.set_chat_tool_pending(None);
                    s.set_chat_tool_running(None);
                    s.set_chat_reasoning(None);
                    s.set_chat_activity_flow(None);
                    s.set_chat_reasoning_compressing(false);
                }
                ChatProgress::ReasoningPartial { text } => {
                    s.set_chat_reasoning(Some(text.clone()));
                }
                ChatProgress::ReasoningCompressing => {
                    s.set_chat_reasoning_compressing(true);
                }
                ChatProgress::ActivityFlow { kind, text } => {
                    s.set_chat_activity_flow(Some(ChatActivityFlow {
                        kind: *kind,
                        text: text.clone(),
                    }));
                }
                ChatProgress::ActivityFlowClear => {
                    s.set_chat_activity_flow(None);
                }
                ChatProgress::ToolPending { label } => {
                    s.set_chat_streaming(None);
                    s.set_chat_tool_pending(Some(label.clone()));
                }
                ChatProgress::AssistantPartial { text } => {
                    s.set_chat_tool_pending(None);
                    if !crate::agent::context::is_tool_result_transcript(text) {
                        s.set_chat_streaming(Some(text.clone()));
                    }
                }
                ChatProgress::ContextSnapshot(snapshot) => {
                    s.set_chat_context(snapshot.clone());
                }
                ChatProgress::ToolStart {
                    name,
                    tool_args_json,
                    ..
                } => {
                    s.set_chat_streaming(None);
                    s.set_chat_tool_pending(None);
                    s.set_chat_reasoning(None);
                    s.set_chat_reasoning_compressing(false);
                    if crate::agent::chat_loop::is_flow_activity_tool(name) {
                        s.set_chat_tool_running(None);
                    } else {
                        s.set_chat_tool_running(Some(name.clone()));
                        let idx = s.chat_lines.len();
                        s.push_chat_line(p.display_line());
                        s.record_chat_tool_args(idx, tool_args_json.clone());
                    }
                }
                ChatProgress::ToolProgress { name, detail }
                    if s.chat_tool_running.as_deref() == Some(name.as_str()) =>
                {
                    s.set_chat_tool_running_detail(Some(detail.clone()));
                }
                ChatProgress::ToolDone {
                    name,
                    output_preview,
                    ..
                } => {
                    s.set_chat_tool_pending(None);
                    s.set_chat_reasoning(None);
                    s.set_chat_reasoning_compressing(false);
                    s.set_chat_tool_running(None);
                    if crate::agent::chat_loop::is_flow_activity_tool(name) {
                        s.set_chat_activity_flow(None);
                    } else {
                        let idx = s.chat_lines.len();
                        s.push_chat_line(p.display_line());
                        s.record_chat_tool_output(idx, output_preview.clone());
                    }
                }
                ChatProgress::ApprovalQueued {
                    approval_id,
                    session_id,
                    tool_name,
                    tool_args_json,
                    description,
                    tool_call_id,
                } => {
                    s.set_chat_streaming(None);
                    s.set_chat_tool_pending(None);
                    s.set_chat_reasoning(None);
                    s.set_chat_reasoning_compressing(false);
                    s.set_chat_tool_running(None);
                    let idx = s.chat_lines.len();
                    s.push_chat_line(p.display_line());
                    s.set_chat_pending_approval(Some(ChatPendingApproval {
                        id: *approval_id,
                        session_id: *session_id,
                        tool_name: tool_name.clone(),
                        tool_args_json: tool_args_json.clone(),
                        tool_call_id: tool_call_id.clone(),
                        line_index: idx,
                    }));
                    if !s.config.chat.auto_approve_mutations {
                        s.open_approval_dialog(
                            *approval_id,
                            tool_name.clone(),
                            description.clone(),
                            Some(tool_args_json.clone()),
                        );
                    }
                }
                ChatProgress::ApprovalResolved {
                    approval_id,
                    approved,
                    detail,
                    ..
                } => {
                    s.set_chat_streaming(None);
                    s.set_chat_tool_pending(None);
                    s.set_chat_reasoning(None);
                    s.set_chat_reasoning_compressing(false);
                    s.set_chat_tool_running(None);
                    s.push_chat_line(p.display_line());
                    s.close_approval_dialog();
                    s.resolve_chat_approval(*approval_id, *approved, detail);
                }
                ChatProgress::UserQuestionQueued {
                    question_id,
                    session_id,
                    question,
                    options,
                    context,
                    tool_call_id,
                    tool_args_json,
                } => {
                    s.set_chat_streaming(None);
                    s.set_chat_tool_pending(None);
                    s.set_chat_reasoning(None);
                    s.set_chat_reasoning_compressing(false);
                    s.set_chat_tool_running(None);
                    let idx = s.chat_lines.len();
                    s.push_chat_line(p.display_line());
                    s.set_chat_pending_user_question(Some(ChatPendingUserQuestion {
                        id: *question_id,
                        session_id: *session_id,
                        question: question.clone(),
                        options: options.clone(),
                        context: context.clone(),
                        tool_call_id: tool_call_id.clone(),
                        tool_args_json: tool_args_json.clone(),
                        line_index: idx,
                    }));
                }
                ChatProgress::UserAnswerResolved {
                    question_id,
                    answer_preview,
                } => {
                    s.set_chat_streaming(None);
                    s.set_chat_tool_pending(None);
                    s.set_chat_reasoning(None);
                    s.set_chat_reasoning_compressing(false);
                    s.set_chat_tool_running(None);
                    // Update the pending ask_user line in place — do not also
                    // push a second "user answered" row (process peek duplicates).
                    s.resolve_chat_user_question(*question_id, answer_preview);
                }
                ChatProgress::ReasoningSummary { body, original, .. } => {
                    s.set_chat_streaming(None);
                    s.set_chat_tool_pending(None);
                    s.set_chat_reasoning_compressing(false);
                    s.set_chat_tool_running(None);
                    if s.chat_busy {
                        let line = p.display_line();
                        if !s.chat_lines.iter().any(|existing| existing == &line) {
                            let idx = s.chat_lines.len();
                            s.push_chat_line(line);
                            if !body.is_empty() {
                                s.record_chat_tool_output(idx, body.clone());
                            }
                            if let Some(orig) = original {
                                if !orig.is_empty() {
                                    s.record_chat_reasoning_original(idx, orig.clone());
                                }
                            }
                        }
                    }
                    s.set_chat_reasoning(None);
                }
                _ if p.show_in_log() => {
                    s.push_chat_line(p.display_line());
                }
                _ => {}
            }
            let status = p.status_text();
            if !status.is_empty() {
                s.status = status;
            }
        }
        AppEvent::ChatReply => {
            s.chat_busy = false;
            s.set_chat_streaming(None);
            s.set_chat_tool_pending(None);
            s.set_chat_tool_running(None);
            s.set_chat_reasoning(None);
            s.set_chat_activity_flow(None);
            s.set_chat_reasoning_compressing(false);
            s.status = "chat ready".into();
        }
    }
}
