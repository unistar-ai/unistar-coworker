use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

mod approval_modal;
mod chat;
mod context_panel;
mod markdown;
mod scroll;
mod spinner;
mod theme;

use theme::ThemePalette;

use approval_modal::{
    draw_approval_modal, handle_approval_modal_key, handle_approval_modal_mouse,
    spawn_approval_decision,
};
use chat::{draw_chat, focus_pane_at, scroll_page_down, scroll_page_up};
use context_panel::{context_status_note, scroll_context_page_down, scroll_context_page_up};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEventKind};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{
    Clear, List, ListItem, ListState, Paragraph, Scrollbar, ScrollbarOrientation, Wrap,
};
use ratatui::DefaultTerminal;
use std::io::{stdout, Write};
use tokio::sync::broadcast;
use unicode_width::UnicodeWidthStr;

use crate::agent::chat_loop::{is_chat_cancelled, ChatProgress};
use crate::app::{
    export_chat_transcript_markdown, load_chat_session_ui, AppEvent, AppState, ChatPaneFocus,
    ChatPendingApproval, SharedState, Tab,
};
use crate::engine::Engine;
use crate::error::Result;
use crate::store::Store;

pub async fn run(
    terminal: &mut DefaultTerminal,
    state: SharedState,
    engine: Arc<Engine>,
    store: Arc<dyn Store>,
    mut events_rx: broadcast::Receiver<AppEvent>,
) -> Result<()> {
    let mut list_state = ListState::default();
    list_state.select(Some(0));

    enable_terminal_modes()?;

    let result = async {
        loop {
            while let Ok(ev) = events_rx.try_recv() {
                apply_event(&state, ev).await;
            }

            {
                let s = state.read().await;
                terminal.draw(|frame| draw_ui(frame, &s, &mut list_state))?;
            }

            if event::poll(Duration::from_millis(200))? {
                match event::read()? {
                    Event::Key(key)
                        if handle_key(key, &state, &engine, &store, &mut list_state).await? =>
                    {
                        break;
                    }
                    Event::Mouse(mouse)
                        if handle_mouse(mouse, terminal, &state, &engine).await? =>
                    {
                        break;
                    }
                    Event::Resize(_, _) => {
                        let mut s = state.write().await;
                        s.invalidate_render_cache();
                    }
                    _ => {}
                }
            }
        }
        Ok::<(), crate::error::CoworkerError>(())
    }
    .await;

    let _ = disable_terminal_modes();
    result?;
    Ok(())
}

/// Alternate scroll (?1007): wheel → cursor keys. Click reporting (?1000): pane focus on click.
fn enable_terminal_modes() -> Result<()> {
    let mut out = stdout();
    out.write_all(b"\x1b[?1007h\x1b[?1000h")?;
    out.flush()?;
    Ok(())
}

fn disable_terminal_modes() -> std::io::Result<()> {
    let mut out = stdout();
    out.write_all(b"\x1b[?1000l\x1b[?1007l")?;
    out.flush()
}

fn is_context_toggle_key(key: &KeyEvent) -> bool {
    !key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char('\\') | KeyCode::Char('＼'))
}

async fn apply_event(state: &SharedState, ev: AppEvent) {
    let mut s = state.write().await;
    match ev {
        AppEvent::StoreUpdated => {
            let prev = s.last_pending_approval_count;
            s.last_pending_approval_count = s.approvals.len();
            if !s.config.chat.auto_approve_mutations
                && s.approval_dialog.is_none()
                && s.approvals.len() > prev
            {
                if let Some(approval) = s.approvals.first().cloned() {
                    s.open_approval_dialog_from(&approval);
                }
            }
            s.status = "store updated".into();
        }
        AppEvent::DigestReady(d) => {
            s.latest_digest = Some(d.clone());
            s.status = if d.summary.complete {
                "digest ready".into()
            } else {
                format!(
                    "digest updating ({} PRs, {} attention)",
                    d.summary.needs_attention + d.summary.ignorable + d.summary.flaky_candidates,
                    d.summary.needs_attention
                )
            };
        }
        AppEvent::LogLine(l) => s.push_log(&l.level, l.message),
        AppEvent::WorkflowStarted { workflow_id } => {
            s.engine_busy = true;
            s.status = format!("running {workflow_id}");
        }
        AppEvent::WorkflowFinished {
            workflow_id,
            ok,
            message,
        } => {
            s.engine_busy = false;
            let status = if ok {
                message.clone()
            } else {
                format!("error: {message}")
            };
            s.status = status;
            s.push_log("info", format!("{workflow_id} finished: {message}"));
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
                    s.set_chat_reasoning_compressing(false);
                }
                ChatProgress::ReasoningPartial { text } => {
                    s.set_chat_reasoning(Some(text.clone()));
                }
                ChatProgress::ReasoningCompressing => {
                    s.set_chat_reasoning_compressing(true);
                }
                ChatProgress::ToolPending { label } => {
                    s.set_chat_streaming(None);
                    s.set_chat_tool_pending(Some(label.clone()));
                }
                ChatProgress::AssistantPartial { text } => {
                    s.set_chat_tool_pending(None);
                    s.set_chat_reasoning(None);
                    if !crate::agent::context::is_tool_result_transcript(text) {
                        s.set_chat_streaming(Some(text.clone()));
                    }
                }
                ChatProgress::ContextSnapshot(snapshot) => {
                    s.set_chat_context(snapshot.clone());
                }
                ChatProgress::ToolStart { name, .. } => {
                    s.set_chat_streaming(None);
                    s.set_chat_tool_pending(None);
                    s.set_chat_reasoning(None);
                    s.set_chat_reasoning_compressing(false);
                    s.set_chat_tool_running(Some(name.clone()));
                    s.push_chat_line(p.display_line());
                }
                ChatProgress::ToolDone { output_preview, .. } => {
                    s.set_chat_tool_pending(None);
                    s.set_chat_reasoning(None);
                    s.set_chat_reasoning_compressing(false);
                    s.set_chat_tool_running(None);
                    let idx = s.chat_lines.len();
                    s.push_chat_line(p.display_line());
                    s.record_chat_tool_output(idx, output_preview.clone());
                }
                ChatProgress::ApprovalQueued {
                    approval_id,
                    session_id,
                    tool_name,
                    tool_args_json,
                    description,
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
                        line_index: idx,
                    }));
                    if !s.config.chat.auto_approve_mutations {
                        s.open_approval_dialog(
                            *approval_id,
                            tool_name.clone(),
                            description.clone(),
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
                ChatProgress::ReasoningSummary { .. } => {
                    s.set_chat_streaming(None);
                    s.set_chat_tool_pending(None);
                    s.set_chat_reasoning(None);
                    s.set_chat_reasoning_compressing(false);
                    s.set_chat_tool_running(None);
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
            s.set_chat_reasoning_compressing(false);
            s.status = "chat ready".into();
        }
    }
}

async fn handle_key(
    key: KeyEvent,
    state: &SharedState,
    engine: &Arc<Engine>,
    store: &Arc<dyn Store>,
    list_state: &mut ListState,
) -> Result<bool> {
    let _store = store;
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return Ok(true);
    }

    let modal_open = {
        let s = state.read().await;
        s.approval_dialog.is_some()
    };
    if modal_open {
        let quit = handle_approval_modal_key(key, state, engine).await;
        return Ok(quit);
    }

    let on_chat_tab = {
        let s = state.read().await;
        s.tab == Tab::Chat
    };
    if on_chat_tab {
        return handle_chat_key(key, state, engine, store, list_state).await;
    }

    match key.code {
        KeyCode::Char('q') => return Ok(true),
        KeyCode::Char('0')
            if {
                let s = state.read().await;
                s.config.chat.enabled
            } =>
        {
            set_tab(state, Tab::Chat, list_state).await;
        }
        KeyCode::Char('1') => set_tab(state, Tab::Dashboard, list_state).await,
        KeyCode::Char('2') => set_tab(state, Tab::Prs, list_state).await,
        KeyCode::Char('3') => set_tab(state, Tab::Approvals, list_state).await,
        KeyCode::Char('4') => set_tab(state, Tab::Logs, list_state).await,
        KeyCode::Char('5') => set_tab(state, Tab::Config, list_state).await,
        KeyCode::Char('6') => set_tab(state, Tab::Flaky, list_state).await,
        KeyCode::Char('7') => {
            let enabled = {
                let s = state.read().await;
                s.config
                    .workflows
                    .get("release-duty")
                    .map(|w| w.enabled)
                    .unwrap_or(false)
            };
            if enabled {
                set_tab(state, Tab::Release, list_state).await;
            }
        }
        KeyCode::Char('8') => {
            let enabled = {
                let s = state.read().await;
                s.config
                    .workflows
                    .get("issue-triage")
                    .map(|w| w.enabled)
                    .unwrap_or(false)
            };
            if enabled {
                set_tab(state, Tab::Issues, list_state).await;
            }
        }
        KeyCode::Char('?') => {
            let mut s = state.write().await;
            if s.config.chat.enabled {
                s.tab = Tab::Chat;
                s.selected_index = 0;
                list_state.select(Some(0));
            }
        }
        KeyCode::Tab => {
            let mut s = state.write().await;
            s.tab = s.tab.next(&s.config);
            s.selected_index = 0;
            list_state.select(Some(0));
        }
        KeyCode::BackTab => {
            let mut s = state.write().await;
            s.tab = s.tab.prev(&s.config);
            s.selected_index = 0;
            list_state.select(Some(0));
        }
        KeyCode::Char('m') => {
            let engine = Arc::clone(engine);
            tokio::spawn(async move {
                let _ = engine.run_workflow("my-pr-brief").await;
            });
        }
        KeyCode::Char('c') if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            let engine = Arc::clone(engine);
            tokio::spawn(async move {
                let _ = engine.run_workflow("ci-efficiency").await;
            });
        }
        KeyCode::Char('/') => {
            let mut s = state.write().await;
            match s.tab {
                Tab::Prs => {
                    s.pr_filter = s.pr_filter.next();
                    s.selected_index = 0;
                    list_state.select(Some(0));
                    s.status = format!("PR filter: {}", s.pr_filter.label());
                }
                Tab::Logs => {
                    s.log_filter = s.log_filter.next();
                    s.selected_index = 0;
                    list_state.select(Some(0));
                    s.status = format!("Log filter: {}", s.log_filter.label());
                }
                _ => {}
            }
        }
        KeyCode::Char('s') => {
            let mut s = state.write().await;
            if s.tab == Tab::Prs {
                s.pr_sort = s.pr_sort.next();
                s.selected_index = 0;
                list_state.select(Some(0));
                s.status = format!("PR sort: {}", s.pr_sort.label());
            }
        }
        KeyCode::Char('A') => {
            let alert_id = {
                let s = state.read().await;
                if s.tab == Tab::Dashboard {
                    dashboard_alert_at(&s, s.selected_index).map(|a| a.id)
                } else {
                    None
                }
            };
            if let Some(id) = alert_id {
                let engine = Arc::clone(engine);
                let state = state.clone();
                tokio::spawn(async move {
                    if let Err(e) = engine.acknowledge_main_alert(&id).await {
                        let mut s = state.write().await;
                        s.push_log("error", format!("ack failed: {e}"));
                    }
                });
            }
        }
        KeyCode::Char('r') => {
            let engine = Arc::clone(engine);
            tokio::spawn(async move {
                let _ = engine.run_workflow("daily-work").await;
            });
        }
        KeyCode::Char('R') => {
            let engine = Arc::clone(engine);
            tokio::spawn(async move {
                let _ = engine.run_workflow("release-duty").await;
            });
        }
        KeyCode::Char('g') => {
            let engine = Arc::clone(engine);
            tokio::spawn(async move {
                let _ = engine.run_workflow("main-guard").await;
            });
        }
        KeyCode::Char('v') => {
            let engine = Arc::clone(engine);
            tokio::spawn(async move {
                let _ = engine.run_workflow("review-radar").await;
            });
        }
        KeyCode::Char('f') => {
            let on_flaky = state.read().await.tab == Tab::Flaky;
            if on_flaky {
                let mut s = state.write().await;
                let repos: Vec<_> = s.config.repos.clone();
                s.flaky_repo_filter = match &s.flaky_repo_filter {
                    None => repos.first().cloned(),
                    Some(cur) => repos
                        .iter()
                        .position(|r| r == cur)
                        .and_then(|i| repos.get(i + 1).cloned()),
                };
                s.selected_index = 0;
                list_state.select(Some(0));
                s.status = format!(
                    "Flaky repo filter: {}",
                    s.flaky_repo_filter.as_deref().unwrap_or("all")
                );
                drop(s);
                let _ = engine.refresh_store().await;
            } else {
                let engine = Arc::clone(engine);
                tokio::spawn(async move {
                    let _ = engine.run_workflow("flaky-govern").await;
                });
            }
        }
        KeyCode::Char('[') => {
            let mut s = state.write().await;
            if s.tab == Tab::Flaky {
                s.flaky_since_days = 7;
                s.selected_index = 0;
                list_state.select(Some(0));
                s.status = "Flaky window: 7d".into();
                drop(s);
                let _ = engine.refresh_store().await;
            }
        }
        KeyCode::Char(']') => {
            let mut s = state.write().await;
            if s.tab == Tab::Flaky {
                s.flaky_since_days = 30;
                s.selected_index = 0;
                list_state.select(Some(0));
                s.status = "Flaky window: 30d".into();
                drop(s);
                let _ = engine.refresh_store().await;
            }
        }
        KeyCode::Char('U') => {
            let fp = {
                let s = state.read().await;
                if s.tab == Tab::Flaky {
                    s.flaky_tests
                        .get(s.selected_index)
                        .map(|t| t.fingerprint.clone())
                } else {
                    None
                }
            };
            if let Some(fingerprint) = fp {
                let engine = Arc::clone(engine);
                let state = state.clone();
                tokio::spawn(async move {
                    match engine.reclassify_flaky(&fingerprint, true).await {
                        Ok(n) => {
                            let mut s = state.write().await;
                            s.status = format!("Marked {n} incident(s) as user-flaky");
                        }
                        Err(e) => {
                            let mut s = state.write().await;
                            s.push_log("error", format!("reclassify: {e}"));
                        }
                    }
                });
            }
        }
        KeyCode::Char('u') => {
            let fp = {
                let s = state.read().await;
                if s.tab == Tab::Flaky {
                    s.flaky_tests
                        .get(s.selected_index)
                        .map(|t| t.fingerprint.clone())
                } else {
                    None
                }
            };
            if let Some(fingerprint) = fp {
                let engine = Arc::clone(engine);
                let state = state.clone();
                tokio::spawn(async move {
                    match engine.reclassify_flaky(&fingerprint, false).await {
                        Ok(n) => {
                            let mut s = state.write().await;
                            s.status = format!("Marked {n} incident(s) as real bug");
                        }
                        Err(e) => {
                            let mut s = state.write().await;
                            s.push_log("error", format!("reclassify: {e}"));
                        }
                    }
                });
            }
        }
        KeyCode::Char('o') => {
            let engine = Arc::clone(engine);
            tokio::spawn(async move {
                let _ = engine.run_workflow("oncall-handoff").await;
            });
        }
        KeyCode::Char('y') => {
            try_decide_approval(state, engine, true).await;
        }
        KeyCode::Char('n') => {
            try_decide_approval(state, engine, false).await;
        }
        KeyCode::Char('{')
            if {
                let s = state.read().await;
                s.tab != Tab::Chat && s.tab != Tab::Flaky
            } =>
        {
            let mut s = state.write().await;
            s.detail_scroll_line = s.detail_scroll_line.saturating_add(DETAIL_SCROLL_PAGE);
        }
        KeyCode::Char('}')
            if {
                let s = state.read().await;
                s.tab != Tab::Chat && s.tab != Tab::Flaky
            } =>
        {
            let mut s = state.write().await;
            s.detail_scroll_line = s.detail_scroll_line.saturating_sub(DETAIL_SCROLL_PAGE);
        }
        KeyCode::Up | KeyCode::Char('k') => {
            let mut s = state.write().await;
            if s.selected_index > 0 {
                s.selected_index -= 1;
                s.reset_detail_scroll();
                list_state.select(Some(s.selected_index));
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            let mut s = state.write().await;
            if s.tab == Tab::Chat {
                return Ok(false);
            }
            let max = list_len(&s).saturating_sub(1);
            if s.selected_index < max {
                s.selected_index += 1;
                s.reset_detail_scroll();
                list_state.select(Some(s.selected_index));
            }
        }
        KeyCode::Enter => {
            try_submit_chat(state, engine, store).await;
        }
        KeyCode::Backspace => {}
        _ => {}
    }
    Ok(false)
}

/// Chat tab: scroll anytime; type into input only while not busy.
async fn handle_chat_key(
    key: KeyEvent,
    state: &SharedState,
    engine: &Arc<Engine>,
    store: &Arc<dyn Store>,
    list_state: &mut ListState,
) -> Result<bool> {
    match key.code {
        KeyCode::Char(c @ '0'..='8') if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            let snapshot = {
                let s = state.read().await;
                (
                    s.chat_input.is_empty(),
                    s.config.chat.enabled,
                    s.config
                        .workflows
                        .get("release-duty")
                        .map(|w| w.enabled)
                        .unwrap_or(false),
                    s.config
                        .workflows
                        .get("issue-triage")
                        .map(|w| w.enabled)
                        .unwrap_or(false),
                )
            };
            if !snapshot.0 {
                let mut s = state.write().await;
                if !s.chat_busy {
                    s.chat_input.push(c);
                }
            } else {
                match c {
                    '0' if snapshot.1 => set_tab(state, Tab::Chat, list_state).await,
                    '1' => set_tab(state, Tab::Dashboard, list_state).await,
                    '2' => set_tab(state, Tab::Prs, list_state).await,
                    '3' => set_tab(state, Tab::Approvals, list_state).await,
                    '4' => set_tab(state, Tab::Logs, list_state).await,
                    '5' => set_tab(state, Tab::Config, list_state).await,
                    '6' => set_tab(state, Tab::Flaky, list_state).await,
                    '7' if snapshot.2 => set_tab(state, Tab::Release, list_state).await,
                    '8' if snapshot.3 => set_tab(state, Tab::Issues, list_state).await,
                    _ => {}
                }
            }
        }
        KeyCode::Tab => {
            let mut s = state.write().await;
            s.tab = s.tab.next(&s.config);
            s.selected_index = 0;
            list_state.select(Some(0));
        }
        KeyCode::BackTab => {
            let mut s = state.write().await;
            s.tab = s.tab.prev(&s.config);
            s.selected_index = 0;
            list_state.select(Some(0));
        }
        KeyCode::PageUp | KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let mut s = state.write().await;
            if s.chat_busy || s.chat_input.is_empty() {
                s.scroll_focused_chat_pane_page_up();
            }
        }
        KeyCode::PageDown | KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let mut s = state.write().await;
            if s.chat_busy || s.chat_input.is_empty() {
                s.scroll_focused_chat_pane_page_down();
            }
        }
        KeyCode::Left
            if {
                let s = state.read().await;
                s.chat_context_visible && s.chat_input.is_empty()
            } =>
        {
            let mut s = state.write().await;
            s.chat_pane_focus = ChatPaneFocus::Messages;
        }
        KeyCode::Right
            if {
                let s = state.read().await;
                s.chat_context_visible && s.chat_input.is_empty()
            } =>
        {
            let mut s = state.write().await;
            s.chat_pane_focus = ChatPaneFocus::Context;
        }
        KeyCode::Up
            if key
                .modifiers
                .intersects(KeyModifiers::ALT | KeyModifiers::CONTROL) =>
        {
            let mut s = state.write().await;
            if s.chat_busy || s.chat_input.is_empty() {
                s.scroll_focused_chat_pane_page_up();
            }
        }
        KeyCode::Down
            if key
                .modifiers
                .intersects(KeyModifiers::ALT | KeyModifiers::CONTROL) =>
        {
            let mut s = state.write().await;
            if s.chat_busy || s.chat_input.is_empty() {
                s.scroll_focused_chat_pane_page_down();
            }
        }
        KeyCode::Up => {
            let mut s = state.write().await;
            if s.chat_busy {
                s.scroll_focused_chat_pane_line_up();
            } else if s.chat_input.is_empty() {
                s.recall_chat_history_up();
            }
        }
        KeyCode::Down => {
            let mut s = state.write().await;
            if s.chat_busy {
                s.scroll_focused_chat_pane_line_down();
            } else if s.chat_input.is_empty() {
                s.recall_chat_history_down();
            }
        }
        KeyCode::Char('j')
            if {
                let s = state.read().await;
                (s.chat_busy || s.chat_input.is_empty())
                    && !key.modifiers.contains(KeyModifiers::CONTROL)
            } =>
        {
            let mut s = state.write().await;
            s.chat_pane_focus = ChatPaneFocus::Messages;
            scroll_page_up(&mut s);
        }
        KeyCode::Char('k')
            if {
                let s = state.read().await;
                (s.chat_busy || s.chat_input.is_empty())
                    && !key.modifiers.contains(KeyModifiers::CONTROL)
            } =>
        {
            let mut s = state.write().await;
            s.chat_pane_focus = ChatPaneFocus::Messages;
            scroll_page_down(&mut s);
        }
        KeyCode::Char('[')
            if {
                let s = state.read().await;
                s.chat_busy || s.chat_input.is_empty()
            } =>
        {
            let mut s = state.write().await;
            s.chat_pane_focus = ChatPaneFocus::Messages;
            scroll_page_up(&mut s);
        }
        KeyCode::Char(']')
            if {
                let s = state.read().await;
                s.chat_busy || s.chat_input.is_empty()
            } =>
        {
            let mut s = state.write().await;
            s.chat_pane_focus = ChatPaneFocus::Messages;
            scroll_page_down(&mut s);
        }
        KeyCode::End => {
            let mut s = state.write().await;
            if s.chat_busy || s.chat_input.is_empty() {
                s.scroll_focused_chat_pane_to_latest();
            }
        }
        KeyCode::Char('\\') | KeyCode::Char('＼') if is_context_toggle_key(&key) => {
            let mut s = state.write().await;
            s.toggle_chat_context_panel();
        }
        KeyCode::Char('{')
            if {
                let s = state.read().await;
                s.chat_context_visible && (s.chat_busy || s.chat_input.is_empty())
            } =>
        {
            let mut s = state.write().await;
            s.chat_pane_focus = ChatPaneFocus::Context;
            scroll_context_page_up(&mut s);
        }
        KeyCode::Char('}')
            if {
                let s = state.read().await;
                s.chat_context_visible && (s.chat_busy || s.chat_input.is_empty())
            } =>
        {
            let mut s = state.write().await;
            s.chat_pane_focus = ChatPaneFocus::Context;
            scroll_context_page_down(&mut s);
        }
        KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => {
            let mut s = state.write().await;
            if !s.chat_busy {
                s.chat_input.push('\n');
                s.chat_history_pos = None;
            }
        }
        KeyCode::Enter => {
            try_submit_chat(state, engine, store).await;
        }
        KeyCode::Esc if state.read().await.chat_busy => {
            engine.request_chat_cancel();
            let mut s = state.write().await;
            s.status = "chat: cancelling…".into();
        }
        KeyCode::Char('o') | KeyCode::Char('O')
            if {
                let s = state.read().await;
                !s.chat_busy && s.chat_input.is_empty() && !s.chat_pane_focus_is_context()
            } =>
        {
            let mut s = state.write().await;
            if s.toggle_last_chat_tool_expand() {
                s.status = "chat: toggled tool output".into();
            }
        }
        KeyCode::Backspace => {
            let mut s = state.write().await;
            if !s.chat_busy {
                s.chat_input.pop();
            }
        }
        KeyCode::Char(c)
            if !key.modifiers.contains(KeyModifiers::CONTROL) && c != '\\' && c != '＼' =>
        {
            let mut s = state.write().await;
            if !s.chat_busy {
                s.chat_input.push(c);
            }
        }
        _ => {}
    }
    Ok(false)
}

async fn try_decide_approval(state: &SharedState, engine: &Arc<Engine>, approve: bool) {
    let id = {
        let s = state.read().await;
        if s.approval_decision_busy() {
            return;
        }
        if let Some(dialog) = &s.approval_dialog {
            if dialog.deciding {
                return;
            }
            Some(dialog.id)
        } else {
            match s.tab {
                Tab::Approvals => s.approvals.get(s.selected_index).map(|a| a.id),
                Tab::Chat if s.chat_input.is_empty() && !s.chat_busy => s.chat_approval_target_id(),
                _ => None,
            }
        }
    };
    let Some(id) = id else {
        return;
    };
    spawn_approval_decision(state, engine, id, approve).await;
}

async fn try_submit_chat(
    state: &SharedState,
    engine: &Arc<Engine>,
    store: &Arc<dyn Store>,
) -> bool {
    let slash = {
        let mut s = state.write().await;
        if s.tab != Tab::Chat || s.chat_busy || s.chat_input.trim().is_empty() {
            None
        } else {
            let msg = s.chat_input.trim().to_string();
            if msg.starts_with('/') {
                s.chat_input.clear();
                s.chat_history_pos = None;
                Some(msg)
            } else {
                None
            }
        }
    };
    if let Some(cmd) = slash {
        return handle_chat_slash_command(state, engine, store, &cmd).await;
    }

    let submit = {
        let mut s = state.write().await;
        if s.tab != Tab::Chat || s.chat_busy || s.chat_input.trim().is_empty() {
            None
        } else {
            let msg = s.chat_input.trim().to_string();
            s.chat_input.clear();
            s.chat_history_pos = None;
            s.push_chat_input_history(msg.clone());
            s.chat_busy = true;
            s.chat_scroll_from_bottom = 0;
            s.chat_context_visible = true;
            s.chat_context_scroll_from_bottom = 0;
            s.chat_pane_focus = ChatPaneFocus::Messages;
            s.status = "chat: waiting for model…".into();
            Some((msg, s.chat_session_id))
        }
    };
    if let Some((msg, session_id)) = submit {
        let engine = Arc::clone(engine);
        let state = state.clone();
        tokio::spawn(async move {
            match engine.run_chat(session_id, &msg).await {
                Ok(result) => {
                    let mut s = state.write().await;
                    s.chat_session_id = Some(result.session_id);
                    s.chat_busy = false;
                    s.set_chat_streaming(None);
                    s.set_chat_tool_pending(None);
                    s.set_chat_tool_running(None);
                    s.set_chat_reasoning(None);
                    s.set_chat_reasoning_compressing(false);
                    s.status = "chat ready".into();
                }
                Err(e) if is_chat_cancelled(&e) => {
                    let mut s = state.write().await;
                    s.chat_busy = false;
                    s.set_chat_streaming(None);
                    s.set_chat_tool_pending(None);
                    s.set_chat_tool_running(None);
                    s.set_chat_reasoning(None);
                    s.set_chat_reasoning_compressing(false);
                    s.status = "chat ready".into();
                }
                Err(e) => {
                    let mut s = state.write().await;
                    s.chat_busy = false;
                    s.set_chat_streaming(None);
                    s.set_chat_tool_pending(None);
                    s.set_chat_tool_running(None);
                    s.set_chat_reasoning(None);
                    s.set_chat_reasoning_compressing(false);
                    s.push_chat_line(format!("error> {e}"));
                    s.status = format!("chat error: {e}");
                }
            }
        });
        true
    } else {
        false
    }
}

async fn handle_chat_slash_command(
    state: &SharedState,
    engine: &Arc<Engine>,
    store: &Arc<dyn Store>,
    cmd: &str,
) -> bool {
    let trimmed = cmd.trim();
    match trimmed {
        "/clear" => {
            let mut s = state.write().await;
            s.clear_chat_transcript();
            s.status = "chat cleared".into();
        }
        "/new" => {
            let mut s = state.write().await;
            s.clear_chat_transcript();
            s.chat_session_id = None;
            s.status = "new chat session".into();
        }
        "/help" => {
            let mut s = state.write().await;
            s.push_chat_line(
                "system> /clear /new — transcript; /sessions — list; /session <id> — switch; /export [path] — save markdown; /approve /deny — pending approval; Shift+Enter — newline; ↑/↓ — history",
            );
            s.status = "chat help".into();
        }
        "/approve" => {
            try_decide_approval(state, engine, true).await;
        }
        "/deny" => {
            try_decide_approval(state, engine, false).await;
        }
        "/sessions" => match store.list_chat_sessions(15).await {
            Ok(sessions) => {
                let mut s = state.write().await;
                if sessions.is_empty() {
                    s.push_chat_line("system> no chat sessions yet");
                } else {
                    s.push_chat_line("system> recent sessions (use /session <id-prefix>):");
                    for sess in sessions {
                        s.push_chat_line(format!(
                            "system> {}  {}  {}",
                            sess.id,
                            sess.created_at.format("%m-%d %H:%M"),
                            trunc(&sess.title, 36)
                        ));
                    }
                }
                s.status = "chat sessions".into();
            }
            Err(e) => {
                let mut s = state.write().await;
                s.status = format!("sessions error: {e}");
            }
        },
        _ if trimmed.starts_with("/session ") => {
            let prefix = trimmed["/session ".len()..].trim();
            if prefix.is_empty() {
                let mut s = state.write().await;
                s.status = "usage: /session <uuid-prefix>".into();
            } else {
                match store.list_chat_sessions(30).await {
                    Ok(sessions) => {
                        let matches: Vec<_> = sessions
                            .iter()
                            .filter(|sess| sess.id.to_string().starts_with(prefix))
                            .collect();
                        let mut s = state.write().await;
                        match matches.len() {
                            0 => s.status = format!("no session matching `{prefix}`"),
                            1 => match load_chat_session_ui(&mut s, store.as_ref(), matches[0].id)
                                .await
                            {
                                Ok(()) => s.status = format!("loaded session {}", matches[0].id),
                                Err(e) => s.status = format!("load failed: {e}"),
                            },
                            n => {
                                s.push_chat_line(format!(
                                    "system> {n} sessions match `{prefix}` — be more specific"
                                ));
                                s.status = "ambiguous session id".into();
                            }
                        }
                    }
                    Err(e) => {
                        let mut s = state.write().await;
                        s.status = format!("sessions error: {e}");
                    }
                }
            }
        }
        _ if trimmed.starts_with("/export") => {
            let path_arg = trimmed.strip_prefix("/export").unwrap_or("").trim();
            let (md, export_path) = {
                let s = state.read().await;
                let md = export_chat_transcript_markdown(&s);
                let path = if path_arg.is_empty() {
                    let sid = s
                        .chat_session_id
                        .map(|id| id.to_string())
                        .unwrap_or_else(|| "draft".into());
                    s.config
                        .storage_path()
                        .join("chat_exports")
                        .join(format!("{sid}.md"))
                } else {
                    PathBuf::from(path_arg)
                };
                (md, path)
            };
            let result = async {
                if let Some(parent) = export_path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&export_path, md)?;
                Ok::<_, std::io::Error>(export_path)
            }
            .await;
            let mut s = state.write().await;
            match result {
                Ok(path) => {
                    s.push_chat_line(format!("system> exported to {}", path.display()));
                    s.status = "chat exported".into();
                }
                Err(e) => s.status = format!("export failed: {e}"),
            }
        }
        _ => {
            let mut s = state.write().await;
            s.status = format!("unknown command: {cmd} (try /help)");
        }
    }
    false
}

const DETAIL_SCROLL_PAGE: u16 = 6;

async fn set_tab(state: &SharedState, tab: Tab, list_state: &mut ListState) {
    let mut s = state.write().await;
    s.tab = tab;
    s.selected_index = 0;
    s.reset_detail_scroll();
    list_state.select(Some(0));
}

fn dashboard_security_count(state: &AppState) -> usize {
    usize::from(state.security_digest_md.is_some())
}

fn dashboard_alert_at(state: &AppState, index: usize) -> Option<&crate::store::MainAlert> {
    state.main_alerts.get(index)
}

fn dashboard_alert_count(state: &AppState) -> usize {
    state.main_alerts.len()
}

fn list_len(s: &AppState) -> usize {
    match s.tab {
        Tab::Chat => s.chat_lines.len().max(1),
        Tab::Dashboard => {
            { dashboard_alert_count(s) + dashboard_security_count(s) + s.digest_history.len() }
                .max(1)
        }
        Tab::Prs => s.sorted_filtered_prs().len().max(1),
        Tab::Approvals => s.approvals.len().max(1),
        Tab::Logs => s.filtered_logs().len().max(1),
        Tab::Config => 4,
        Tab::Flaky => s.flaky_tests.len().max(1),
        Tab::Release => s.backport_queue.len().max(1),
        Tab::Issues => s.issues.len().max(1),
    }
}

fn ui_content_area(full: Rect) -> Rect {
    Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(full)[2]
}

async fn handle_mouse(
    mouse: crossterm::event::MouseEvent,
    terminal: &DefaultTerminal,
    state: &SharedState,
    engine: &Arc<Engine>,
) -> Result<bool> {
    let modal_open = {
        let s = state.read().await;
        s.approval_dialog.is_some()
    };
    if modal_open {
        let size = terminal.size().map_err(|e| {
            crate::error::CoworkerError::Other(anyhow::anyhow!("terminal size: {e}"))
        })?;
        let frame_area = Rect::new(0, 0, size.width, size.height);
        handle_approval_modal_mouse(mouse, frame_area, state, engine).await;
        return Ok(false);
    }

    match mouse.kind {
        MouseEventKind::ScrollUp => {
            let mut s = state.write().await;
            if s.tab == Tab::Chat && (s.chat_busy || s.chat_input.is_empty()) {
                s.scroll_focused_chat_pane_line_up();
            }
        }
        MouseEventKind::ScrollDown => {
            let mut s = state.write().await;
            if s.tab == Tab::Chat && (s.chat_busy || s.chat_input.is_empty()) {
                s.scroll_focused_chat_pane_line_down();
            }
        }
        MouseEventKind::Down(MouseButton::Left) => {
            let size = terminal.size().map_err(|e| {
                crate::error::CoworkerError::Other(anyhow::anyhow!("terminal size: {e}"))
            })?;
            let content = ui_content_area(Rect::new(0, 0, size.width, size.height));
            let mut s = state.write().await;
            if s.tab == Tab::Chat && s.chat_context_visible {
                if let Some(focus) = focus_pane_at(content, true, mouse.column, mouse.row) {
                    s.chat_pane_focus = focus;
                }
            }
        }
        _ => {}
    }
    Ok(false)
}

fn draw_ui(frame: &mut ratatui::Frame, state: &AppState, list_state: &mut ListState) {
    let th = ThemePalette::from_mode(state.config.tui.theme);
    frame.render_widget(Clear, frame.area());
    frame.render_widget(
        Paragraph::new("").style(Style::default().bg(th.bg)),
        frame.area(),
    );

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(frame.area());

    draw_header(frame, chunks[0], state, th);
    draw_hints(frame, chunks[1], state, th);

    if state.tab == Tab::Chat {
        draw_chat(frame, chunks[2], state);
    } else {
        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(38), Constraint::Percentage(62)])
            .split(chunks[2]);

        draw_list(frame, body[0], state, list_state, th);
        draw_detail(frame, body[1], state, th);
    }
    draw_status(frame, chunks[3], state, th);
    if state.approval_dialog.is_some() {
        draw_approval_modal(frame, state, th);
    }
}

fn draw_header(frame: &mut ratatui::Frame, area: Rect, state: &AppState, th: ThemePalette) {
    let tabs_list: Vec<Span> = Tab::all_for_config(&state.config)
        .iter()
        .enumerate()
        .flat_map(|(i, t)| {
            let mut spans = Vec::new();
            if i > 0 {
                spans.push(theme::tab_separator(th));
            }
            spans.push(theme::tab_spans(th, t.label(), *t == state.tab));
            spans
        })
        .collect();

    let block = theme::header_block(th);
    let inner = block.inner(area);
    frame.render_widget(Paragraph::new("").block(block), area);
    frame.render_widget(
        Paragraph::new(Line::from(tabs_list)).style(Style::default().bg(th.surface)),
        inner,
    );
}

fn draw_hints(frame: &mut ratatui::Frame, area: Rect, state: &AppState, th: ThemePalette) {
    let hint = match state.tab {
        Tab::Chat => {
            let approval_hint = if state.approval_dialog.is_some() {
                "  popup: click Approve/Deny · ←/→ · Enter"
            } else if state.can_decide_approval_inline() && state.chat_input.is_empty() {
                "  y: approve  n: deny"
            } else {
                ""
            };
            if state.chat_context_visible {
                if state.chat_busy {
                    format!(
                        "click/←/→: focus  ↑/↓: scroll  \\: hide ctx  Esc: cancel  End: latest{approval_hint}"
                    )
                } else {
                    format!(
                        "click/←/→: focus  ↑/↓: scroll  o: expand tool (input empty)  j/k: msgs  \\: ctx  End: latest{approval_hint}"
                    )
                }
            } else if state.chat_busy {
                format!("Enter: send  Esc: cancel  j/k: scroll  /help  \\: ctx{approval_hint}")
            } else {
                format!(
                    "Enter: send  ↑/↓: history  Shift+Enter: newline  j/k: scroll  /help  \\: ctx{approval_hint}"
                )
            }
        }
        Tab::Dashboard => {
            "r: daily  R: release  g: guard  v: radar  m: my-prs  c: ci-eff  f: flaky  o: oncall  A: ack  {/}: detail scroll".into()
        }
        Tab::Prs => return frame.render_widget(
            Paragraph::new(theme::hint_bar(
                th,
                &format!(
                    "filter: {} (/)  sort: {} (s)  j/k scroll  q: quit",
                    state.pr_filter.label(),
                    state.pr_sort.label()
                ),
            ))
            .style(Style::default().bg(th.title_bg)),
            area,
        ),
        Tab::Approvals => "y: approve (runs MCP)  n: deny  q: quit".into(),
        Tab::Logs => return frame.render_widget(
            Paragraph::new(theme::hint_bar(
                th,
                &format!(
                    "filter: {} (/)  j/k scroll  q: quit",
                    state.log_filter.label()
                ),
            ))
            .style(Style::default().bg(th.title_bg)),
            area,
        ),
        Tab::Release => "backport queue  j/k scroll  q: quit".into(),
        Tab::Issues => "open issues  j/k scroll  q: quit".into(),
        Tab::Flaky => "[/]: 7/30d  f: repo filter  u/U: reclassify  j/k scroll  q: quit".into(),
        _ => "j/k: scroll  {/}: detail  Tab: next  q: quit".into(),
    };
    frame.render_widget(
        Paragraph::new(theme::hint_bar(th, &hint)).style(Style::default().bg(th.title_bg)),
        area,
    );
}

fn draw_list(
    frame: &mut ratatui::Frame,
    area: Rect,
    state: &AppState,
    list_state: &mut ListState,
    th: ThemePalette,
) {
    let items: Vec<ListItem> = match state.tab {
        Tab::Chat => vec![ListItem::new("use chat pane →")],
        Tab::Dashboard => {
            let mut items: Vec<ListItem> = state
                .main_alerts
                .iter()
                .map(|a| {
                    ListItem::new(format!(
                        "! {}@{} — {} fail(s) run {}",
                        a.repo, a.branch, a.consecutive_failures, a.latest_run_id
                    ))
                })
                .collect();
            if state.security_digest_md.is_some() {
                items.push(ListItem::new("🔒 Security digest"));
            }
            if state.digest_history.is_empty() && items.is_empty() {
                items.push(ListItem::new("No digest — press r"));
            } else {
                items.extend(state.digest_history.iter().map(|d| {
                    ListItem::new(format!(
                        "▸ {} — attention:{} flaky:{} policy:{} ok:{} ({})",
                        d.date,
                        d.summary.needs_attention,
                        d.summary.flaky_candidates,
                        d.summary.policy_gates,
                        d.summary.ignorable,
                        d.summary.duration_label()
                    ))
                }));
            }
            items
        }
        Tab::Prs => {
            let filtered = state.sorted_filtered_prs();
            if filtered.is_empty() {
                vec![ListItem::new(format!(
                    "No PRs ({}) — run daily-work or my-pr-brief",
                    state.pr_filter.label()
                ))]
            } else {
                filtered
                    .into_iter()
                    .map(|p| {
                        ListItem::new(Line::from(vec![
                            Span::styled(
                                format!("{} ", theme::pr_ci_glyph(&p.ci_summary)),
                                theme::ci_status_style(th, &p.ci_summary),
                            ),
                            Span::styled(
                                format!("{} ", theme::pr_review_glyph(&p.review_summary)),
                                theme::review_status_style(th, &p.review_summary),
                            ),
                            Span::styled(
                                format!("#{} ", p.number),
                                Style::default().fg(th.accent).add_modifier(Modifier::BOLD),
                            ),
                            Span::styled(trunc(&p.title, 24), Style::default().fg(th.text)),
                            Span::styled(format!(" [{}] ", p.repo), Style::default().fg(th.muted)),
                            Span::styled(
                                p.ci_summary.clone(),
                                theme::ci_status_style(th, &p.ci_summary),
                            ),
                        ]))
                    })
                    .collect()
            }
        }
        Tab::Approvals => {
            if state.approvals.is_empty() {
                vec![ListItem::new("No pending approvals")]
            } else {
                state
                    .approvals
                    .iter()
                    .map(|a| ListItem::new(format!("{:?} {}", a.kind, trunc(&a.description, 44))))
                    .collect()
            }
        }
        Tab::Logs => {
            let filtered = state.filtered_logs();
            if filtered.is_empty() {
                vec![ListItem::new(format!(
                    "No logs ({})",
                    state.log_filter.label()
                ))]
            } else {
                filtered
                    .iter()
                    .rev()
                    .take(80)
                    .map(|l| {
                        ListItem::new(Line::from(vec![
                            Span::styled(
                                format!("{} ", l.ts.format("%H:%M:%S")),
                                Style::default().fg(th.muted),
                            ),
                            Span::styled(
                                format!("[{}] ", l.level.to_ascii_uppercase()),
                                theme::log_level_style(th, &l.level),
                            ),
                            Span::styled(trunc(&l.message, 44), Style::default().fg(th.text)),
                        ]))
                    })
                    .collect()
            }
        }
        Tab::Config => vec![
            ListItem::new(format!("config: {}", state.config_path)),
            ListItem::new(format!("repos: {}", state.config.repos.join(", "))),
            ListItem::new(format!("storage: {:?}", state.config.storage.backend)),
            ListItem::new(format!("llm: {}", state.config.llm.model)),
            ListItem::new(format!("tui theme: {:?}", state.config.tui.theme)),
        ],
        Tab::Flaky => {
            if state.flaky_tests.is_empty() {
                vec![ListItem::new("No flaky tests")]
            } else {
                state
                    .flaky_tests
                    .iter()
                    .map(|t| {
                        ListItem::new(format!(
                            "{} x{} {}",
                            t.test_name.as_deref().unwrap_or(&t.workflow),
                            t.incident_count,
                            t.repo
                        ))
                    })
                    .collect()
            }
        }
        Tab::Release => {
            if state.backport_queue.is_empty() {
                vec![ListItem::new("No backport queue — run release-duty")]
            } else {
                state
                    .backport_queue
                    .iter()
                    .map(|b| {
                        ListItem::new(format!(
                            "#{} → {} [{:?}]",
                            b.pr_number, b.target_branch, b.status
                        ))
                    })
                    .collect()
            }
        }
        Tab::Issues => {
            if state.issues.is_empty() {
                vec![ListItem::new("No issues — run issue-triage")]
            } else {
                state
                    .issues
                    .iter()
                    .map(|i| {
                        ListItem::new(format!(
                            "{}#{} {} @{} [{}]",
                            i.repo,
                            i.number,
                            trunc(&i.title, 32),
                            i.author,
                            i.labels
                        ))
                    })
                    .collect()
            }
        }
    };

    let list = List::new(items)
        .block(theme::list_block(th, "List"))
        .highlight_style(
            Style::default()
                .bg(th.tab_active_bg)
                .fg(th.accent)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▸ ");
    frame.render_stateful_widget(list, area, list_state);
}

fn draw_detail(frame: &mut ratatui::Frame, area: Rect, state: &AppState, th: ThemePalette) {
    let (body, render_markdown) = detail_body(state);
    draw_detail_pane(
        frame,
        area,
        th,
        &body,
        render_markdown,
        state.detail_scroll_line,
    );
}

struct DetailBody {
    text: String,
    markdown: bool,
}

fn detail_body(state: &AppState) -> (String, bool) {
    let view = match state.tab {
        Tab::Chat => DetailBody {
            text: String::new(),
            markdown: false,
        },
        Tab::Dashboard => {
            let alert_n = dashboard_alert_count(state);
            let sec_n = dashboard_security_count(state);
            if state.selected_index < alert_n {
                DetailBody {
                    text: dashboard_alert_at(state, state.selected_index).map_or_else(
                        || "Select an alert".into(),
                        |a| {
                            format!(
                                "Main alert\nrepo: {}@{}\nconsecutive failures: {}\nrun: {} ({})\nconclusion: {}\n\nPress A to acknowledge",
                                a.repo,
                                a.branch,
                                a.consecutive_failures,
                                a.latest_run_id,
                                a.latest_workflow,
                                a.conclusion
                            )
                        },
                    ),
                    markdown: false,
                }
            } else if state.selected_index < alert_n + sec_n {
                DetailBody {
                    text: state
                        .security_digest_md
                        .clone()
                        .unwrap_or_else(|| "Run security-digest workflow.".into()),
                    markdown: true,
                }
            } else {
                let digest_idx = state.selected_index - alert_n - sec_n;
                if digest_idx == 0 {
                    DetailBody {
                        text: state
                            .latest_digest
                            .as_ref()
                            .map(|d| d.body_md.clone())
                            .unwrap_or_else(|| "Press r to run daily-work.".into()),
                        markdown: true,
                    }
                } else if let Some(meta) = state.digest_history.get(digest_idx) {
                    DetailBody {
                        text: state
                            .digest_bodies
                            .get(&meta.date)
                            .cloned()
                            .unwrap_or_else(|| {
                                format!(
                                    "Digest {}\nattention: {}  flaky: {}  policy: {}  ok: {}\nrun time: {}",
                                    meta.date,
                                    meta.summary.needs_attention,
                                    meta.summary.flaky_candidates,
                                    meta.summary.policy_gates,
                                    meta.summary.ignorable,
                                    meta.summary.duration_label()
                                )
                            }),
                        markdown: state.digest_bodies.contains_key(&meta.date),
                    }
                } else {
                    DetailBody {
                        text: "Press r to run daily-work.".into(),
                        markdown: false,
                    }
                }
            }
        }
        Tab::Prs => DetailBody {
            text: state
                .sorted_filtered_prs()
                .get(state.selected_index)
                .map(|p| {
                    format!(
                        "#{} {} @{} \nrepo: {}\nci: {} review: {}\n\n{}",
                        p.number,
                        p.title,
                        p.author,
                        p.repo,
                        p.ci_summary,
                        p.review_summary,
                        p.triage_note.as_deref().unwrap_or("(no triage yet)")
                    )
                })
                .unwrap_or_else(|| "Select a PR".into()),
            markdown: false,
        },
        Tab::Approvals => DetailBody {
            text: state
                .selected_approval()
                .map(format_approval_detail)
                .unwrap_or_else(|| "Select an approval".into()),
            markdown: false,
        },
        Tab::Logs => {
            let logs: Vec<_> = state.filtered_logs().into_iter().rev().collect();
            DetailBody {
                text: logs
                    .get(state.selected_index)
                    .map(|l| {
                        format!(
                            "[{}] {}\n{}",
                            l.level,
                            l.ts.format("%Y-%m-%d %H:%M:%S"),
                            l.message
                        )
                    })
                    .unwrap_or_else(|| format!("No logs ({})", state.log_filter.label())),
                markdown: false,
            }
        }
        Tab::Config => DetailBody {
            text: format!(
                "MCP: {} ({})\nLLM: {} ({})\nEnabled workflows:\n  {}",
                state.config.mcp.command,
                if state.mcp_ok { "ok" } else { "offline" },
                state.config.llm.base_url,
                if state.llm_ok { "ok" } else { "offline" },
                state
                    .config
                    .workflows
                    .iter()
                    .filter(|(_, w)| w.enabled)
                    .map(|(k, _)| k.as_str())
                    .collect::<Vec<_>>()
                    .join("\n  ")
            ),
            markdown: false,
        },
        Tab::Flaky => DetailBody {
            text: state
                .flaky_tests
                .get(state.selected_index)
                .map(|t| {
                    let rate = if t.rerun_attempts == 0 {
                        "—".into()
                    } else {
                        format!(
                            "{:.0}%",
                            100.0 * f64::from(t.rerun_successes) / f64::from(t.rerun_attempts)
                        )
                    };
                    let quarantine = if t.incident_count >= 5
                        && t.rerun_attempts > 0
                        && t.rerun_successes * 2 < t.rerun_attempts
                    {
                        "\n\n⚠ Quarantine candidate (high count, low rerun success)"
                    } else {
                        ""
                    };
                    format!(
                        "fingerprint: {}\nrepo: {}\nworkflow: {}\ncount: {}\nrerun: {}/{} ({rate})\nlast: {}{quarantine}\n\nu: mark real bug  U: mark flaky",
                        t.fingerprint,
                        t.repo,
                        t.workflow,
                        t.incident_count,
                        t.rerun_successes,
                        t.rerun_attempts,
                        t.last_seen
                    )
                })
                .unwrap_or_else(|| {
                    format!(
                        "Flaky report ({}d, repo: {}) — run flaky-govern or daily-work",
                        state.flaky_since_days,
                        state.flaky_repo_filter.as_deref().unwrap_or("all")
                    )
                }),
            markdown: false,
        },
        Tab::Release => DetailBody {
            text: state
                .selected_backport()
                .map(|b| {
                    format!(
                        "#{} {}\nrepo: {}\ntarget: {}\nstatus: {:?}\ncreated: {}",
                        b.pr_number, b.pr_title, b.repo, b.target_branch, b.status, b.created_at
                    )
                })
                .unwrap_or_else(|| "Select a backport item".into()),
            markdown: false,
        },
        Tab::Issues => DetailBody {
            text: state
                .selected_issue()
                .map(|i| {
                    format!(
                        "#{} {}\nrepo: {}\nauthor: {}\nlabels: {}\nupdated: {}\n\n{}",
                        i.number,
                        i.title,
                        i.repo,
                        i.author,
                        i.labels,
                        i.updated_at,
                        i.triage_note.as_deref().unwrap_or("")
                    )
                })
                .unwrap_or_else(|| "Select an issue".into()),
            markdown: false,
        },
    };
    (view.text, view.markdown)
}

fn format_approval_detail(a: &crate::store::Approval) -> String {
    let mut lines = vec![
        format!("Kind:     {:?}", a.kind),
        format!("Status:   {:?}", a.status),
        format!("Repo:     {}", a.repo),
        format!("Created:  {}", a.created_at.format("%Y-%m-%d %H:%M:%S")),
        String::new(),
        a.description.clone(),
    ];
    if let Some(n) = a.pr_number {
        lines.push(format!("PR:       #{n}"));
    }
    if let Some(run) = a.run_id {
        lines.push(format!("Run:      {run}"));
    }
    if let Some(ref branch) = a.target_branch {
        lines.push(format!("Branch:   {branch}"));
    }
    if let Some(ref body) = a.comment_body {
        lines.push(String::new());
        lines.push("Comment body:".into());
        lines.push(body.clone());
    }
    if let Some(id) = a.incident_id {
        lines.push(format!("Incident: {id}"));
    }
    lines.join("\n")
}

fn draw_detail_pane(
    frame: &mut ratatui::Frame,
    area: Rect,
    th: ThemePalette,
    body: &str,
    render_markdown: bool,
    scroll_line: u16,
) {
    let block = theme::detail_block(th);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let base = Style::default().fg(th.text);
    let lines: Vec<Line> = if render_markdown {
        markdown::markdown_to_lines_in_width(th, body, base, Some(inner.width.max(1) as usize))
    } else {
        body.lines()
            .map(|line| {
                if line.is_empty() {
                    Line::from("")
                } else {
                    Line::from(Span::styled(line.to_string(), base))
                }
            })
            .collect()
    };
    let lines = if lines.is_empty() {
        vec![Line::from("")]
    } else {
        lines
    };
    let width = inner.width.max(1);
    let visible = inner.height.max(1);
    let total = Paragraph::new(Text::from(lines.clone()))
        .wrap(Wrap { trim: false })
        .line_count(width) as u16;
    let max_scroll = total.saturating_sub(visible);
    let scroll = scroll_line.min(max_scroll);
    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .style(Style::default().bg(th.panel))
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0)),
        inner,
    );
    if total > visible {
        let mut sb_state = scroll::paragraph_scrollbar_state(total, visible, scroll);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .thumb_symbol("█")
                .track_symbol(Some("░"))
                .thumb_style(Style::default().fg(th.accent_dim))
                .track_style(Style::default().fg(th.muted)),
            inner,
            &mut sb_state,
        );
    }
}

fn draw_status(frame: &mut ratatui::Frame, area: Rect, state: &AppState, th: ThemePalette) {
    let busy = state.engine_busy || state.chat_busy;
    let alert_note = if state.main_alerts.is_empty() {
        String::new()
    } else {
        format!(" │ main alerts: {}", state.main_alerts.len())
    };
    let ctx_note = context_status_note(state);
    let phase_note = state
        .chat_turn_phase()
        .map(|p| format!(" │ phase: {p}"))
        .unwrap_or_default();
    let line = theme::status_line(
        th,
        busy,
        &state.status,
        state.mcp_ok,
        state.llm_ok,
        &format!("{ctx_note}{phase_note}{alert_note}"),
    );
    frame.render_widget(
        Paragraph::new(line).style(Style::default().bg(th.surface)),
        area,
    );
}

fn trunc(s: &str, max: usize) -> String {
    if s.width() <= max {
        return s.to_string();
    }
    let mut out = String::new();
    let mut w = 0;
    for ch in s.chars() {
        let cw = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if w + cw > max.saturating_sub(1) {
            break;
        }
        out.push(ch);
        w += cw;
    }
    out.push('…');
    out
}
