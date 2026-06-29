use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

mod approval_modal;
mod chat;
mod clipboard;
mod context_panel;
mod detail_cache;
mod digest_nav;
mod markdown;
mod scroll;
mod spinner;
mod theme;

use theme::ThemePalette;

use approval_modal::{draw_approval_modal, handle_approval_modal_key, handle_approval_modal_mouse};
use chat::{draw_chat, focus_pane_at, scroll_page_down, scroll_page_up};
use context_panel::{
    context_status_spans, scroll_context_page_down, scroll_context_page_up, store_status_spans,
};
use detail_cache::{cached_detail_markdown_lines, detail_body_cache_key};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEventKind};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{
    Clear, List, ListItem, ListState, Paragraph, Scrollbar, ScrollbarOrientation, Wrap,
};
use ratatui::DefaultTerminal;
use std::io::{stdout, Write};
use std::time::Instant;
use tokio::sync::broadcast;
use unicode_width::UnicodeWidthStr;

use crate::agent::chat_loop::is_chat_cancelled;
use crate::app::{
    apply_event, export_chat_transcript_markdown, hydrate_from_store, load_chat_session_ui,
    spawn_approval_decision, AppEvent, AppState, ChatPaneFocus, DashboardSection, SharedState, Tab,
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
    spinner::reset_session();

    const ATTACH_POLL: Duration = Duration::from_secs(2);
    let mut last_attach_poll = Instant::now();

    let result = async {
        loop {
            while let Ok(ev) = events_rx.try_recv() {
                apply_event(&state, ev).await;
            }

            if {
                let s = state.read().await;
                s.attach_mode
            } && last_attach_poll.elapsed() >= ATTACH_POLL
            {
                let prev = state.read().await.approvals.len();
                if hydrate_from_store(&state, store.as_ref()).await.is_ok() {
                    let mut s = state.write().await;
                    s.maybe_notify_new_workflow_approvals(prev);
                }
                last_attach_poll = Instant::now();
            }

            maybe_request_pr_overview(&state, &engine).await;

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
                        if handle_mouse(mouse, terminal, &state, &engine, &mut list_state)
                            .await? =>
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
            set_tab(state, Tab::Prs, list_state).await;
        }
        KeyCode::Char('c') if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            set_tab(state, Tab::Chat, list_state).await;
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
        KeyCode::Char('r') => {
            let engine = Arc::clone(engine);
            tokio::spawn(async move {
                let _ = engine.run_workflow("daily-work").await;
            });
        }
        KeyCode::Char('R') => {
            let on_config = state.read().await.tab == Tab::Config;
            if on_config {
                let engine = Arc::clone(engine);
                let state = state.clone();
                tokio::spawn(async move {
                    engine.refresh_connectivity_probes().await;
                    let mut s = state.write().await;
                    s.status = "connectivity probes refreshed".into();
                });
            } else {
                let engine = Arc::clone(engine);
                tokio::spawn(async move {
                    let _ = engine.run_workflow("review-radar").await;
                });
            }
        }
        KeyCode::Char('v') => {
            let engine = Arc::clone(engine);
            tokio::spawn(async move {
                let _ = engine.run_workflow("review-radar").await;
            });
        }
        KeyCode::Char('z') => {
            let mut s = state.write().await;
            if s.tab == Tab::Dashboard {
                if let Some(kind) = dashboard_row_kind_at(&s, s.selected_index) {
                    if let Some(section) = dashboard_section_for_row(&kind) {
                        s.toggle_dashboard_section_fold(section);
                        clamp_dashboard_selection(&mut s);
                        list_state.select(Some(s.selected_index));
                        s.status = format!(
                            "Dashboard {} {}",
                            section.label(),
                            if s.dashboard_fold.is_collapsed(section) {
                                "collapsed"
                            } else {
                                "expanded"
                            }
                        );
                    }
                }
            }
        }
        KeyCode::Char('Z') => {
            let mut s = state.write().await;
            if s.tab == Tab::Dashboard {
                if let Some(title) = cycle_digest_section_fold(&mut s) {
                    let folded = s.digest_folded_sections.contains(&title);
                    s.reset_detail_scroll();
                    s.status = format!(
                        "digest section '{title}' {}",
                        if folded { "folded" } else { "expanded" }
                    );
                }
            }
        }
        KeyCode::Char('t') => {
            let triage = {
                let s = state.read().await;
                if s.tab == Tab::Prs && s.github_ok {
                    s.sorted_filtered_prs()
                        .get(s.selected_index)
                        .map(|pr| (pr.repo.clone(), pr.number))
                } else {
                    None
                }
            };
            if let Some((repo, number)) = triage {
                let engine = Arc::clone(engine);
                engine.spawn_triage_pr(repo, number);
            }
        }
        KeyCode::Char('p') => {
            jump_to_pr_from_context(state, list_state).await;
        }
        KeyCode::Char('o') => {
            let pr_refresh = {
                let s = state.read().await;
                if s.tab == Tab::Prs && s.github_ok {
                    s.sorted_filtered_prs()
                        .get(s.selected_index)
                        .map(|pr| (pr.repo.clone(), pr.number))
                } else {
                    None
                }
            };
            if let Some((repo, number)) = pr_refresh {
                let key = AppState::pr_overview_key(&repo, number);
                {
                    let mut s = state.write().await;
                    s.pr_overview_cache.remove(&key);
                    s.pr_overview_fetching = Some(key.clone());
                    s.status = format!("refreshing overview {repo}#{number}");
                }
                let engine = Arc::clone(engine);
                tokio::spawn(async move {
                    engine.fetch_pr_overview(repo, number).await;
                });
            }
        }
        KeyCode::Char('y') => {
            try_decide_approval(state, engine, true).await;
        }
        KeyCode::Char('n') => {
            let tab = state.read().await.tab;
            if tab == Tab::Dashboard {
                let mut s = state.write().await;
                if let Some(msg) = cycle_dashboard_digest_pr(&mut s, 1) {
                    s.status = msg;
                }
            } else if tab == Tab::Approvals {
                try_decide_approval(state, engine, false).await;
            }
        }
        KeyCode::Char('N') => {
            let mut s = state.write().await;
            if s.tab == Tab::Dashboard {
                if let Some(msg) = cycle_dashboard_digest_pr(&mut s, -1) {
                    s.status = msg;
                }
            }
        }
        KeyCode::Char('{')
            if {
                let s = state.read().await;
                s.tab != Tab::Chat
            } =>
        {
            let mut s = state.write().await;
            s.detail_scroll_line = s.detail_scroll_line.saturating_add(DETAIL_SCROLL_PAGE);
        }
        KeyCode::Char('}')
            if {
                let s = state.read().await;
                s.tab != Tab::Chat
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
            let jump_digest = {
                let s = state.read().await;
                s.tab == Tab::Dashboard
                    && matches!(
                        dashboard_row_kind_at(&s, s.selected_index),
                        Some(DashboardRowKind::Digest(_))
                    )
            };
            if jump_digest {
                jump_to_pr_from_context(state, list_state).await;
            } else {
                try_submit_chat(state, engine, store).await;
            }
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
        KeyCode::Char(c @ '0'..='5') if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            let snapshot = {
                let s = state.read().await;
                (s.chat_input.is_empty(), s.config.chat.enabled)
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
                    s.set_chat_activity_flow(None);
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
                    s.set_chat_activity_flow(None);
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
                    s.set_chat_activity_flow(None);
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
            s.reset_chat_session();
            s.status = "chat cleared".into();
        }
        "/new" => {
            let mut s = state.write().await;
            s.reset_chat_session();
            s.status = "new chat session".into();
        }
        "/help" => {
            let mut s = state.write().await;
            s.push_chat_line(
                "system> **Slash**: /clear /new — reset transcript + LLM context; /sessions /session <id> — history; /export [path] — markdown; /approve /deny — pending approval",
            );
            s.push_chat_line(
                "system> **Chat**: Enter send; Shift+Enter newline; ↑/↓ input history; j/k scroll msgs; o expand tool (input empty); Esc cancel; End latest; \\ toggle ctx panel",
            );
            s.push_chat_line(
                "system> **Tabs**: Tab/BackTab cycle; 1–8 jump; q quit; Ctrl+c quit; ? open Chat; Dashboard r/R/g/v/m/c/f/o/A Enter/p open PR; PRs / filter s sort t triage; Flaky u/U reclassify",
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

#[derive(Debug, Clone)]
enum DashboardRowKind {
    SectionHeader(DashboardSection),
    Digest(usize),
    Placeholder,
}

struct DashboardRow {
    kind: DashboardRowKind,
    label: String,
}

fn dashboard_rows(state: &AppState) -> Vec<DashboardRow> {
    let fold = state.dashboard_fold;
    let mut rows = Vec::new();
    let digest_n = state.digest_history.len();
    rows.push(DashboardRow {
        kind: DashboardRowKind::SectionHeader(DashboardSection::Digests),
        label: format!(
            "{} Daily digests ({digest_n})",
            if fold.digests { "▸" } else { "▾" }
        ),
    });
    if !fold.digests {
        if digest_n == 0 {
            rows.push(DashboardRow {
                kind: DashboardRowKind::Placeholder,
                label: "  No digest — press r".into(),
            });
        } else {
            for (i, d) in state.digest_history.iter().enumerate() {
                let updating = if d.summary.complete { "" } else { " ◷" };
                rows.push(DashboardRow {
                    kind: DashboardRowKind::Digest(i),
                    label: format!(
                        "  ▸ {} — attention:{} flaky:{} policy:{} ok:{}{} ({})",
                        d.date,
                        d.summary.needs_attention,
                        d.summary.flaky_candidates,
                        d.summary.policy_gates,
                        d.summary.ignorable,
                        updating,
                        d.summary.duration_label()
                    ),
                });
            }
        }
    }

    if rows.is_empty() {
        rows.push(DashboardRow {
            kind: DashboardRowKind::Placeholder,
            label: "No digest — press r".into(),
        });
    }
    rows
}

fn dashboard_row_kind_at(state: &AppState, index: usize) -> Option<DashboardRowKind> {
    dashboard_rows(state).get(index).map(|row| row.kind.clone())
}

fn dashboard_section_for_row(kind: &DashboardRowKind) -> Option<DashboardSection> {
    match kind {
        DashboardRowKind::SectionHeader(section) => Some(*section),
        DashboardRowKind::Digest(_) => Some(DashboardSection::Digests),
        DashboardRowKind::Placeholder => None,
    }
}

fn clamp_dashboard_selection(state: &mut AppState) {
    let max = dashboard_rows(state).len().saturating_sub(1);
    if state.selected_index > max {
        state.selected_index = max;
        state.reset_detail_scroll();
    }
}

fn dashboard_digest_body(state: &AppState, digest_idx: usize) -> Option<String> {
    if digest_idx == 0 {
        state.latest_digest.as_ref().map(|d| d.body_md.clone())
    } else {
        state.digest_history.get(digest_idx).and_then(|meta| {
            state.digest_bodies.get(&meta.date).cloned().or_else(|| {
                Some(format!(
                    "Digest {}\nattention: {}  flaky: {}  policy: {}  ok: {}\nrun time: {}",
                    meta.date,
                    meta.summary.needs_attention,
                    meta.summary.flaky_candidates,
                    meta.summary.policy_gates,
                    meta.summary.ignorable,
                    meta.summary.duration_label()
                ))
            })
        })
    }
}

fn cycle_digest_section_fold(state: &mut AppState) -> Option<String> {
    let kind = dashboard_row_kind_at(state, state.selected_index)?;
    let digest_idx = match kind {
        DashboardRowKind::Digest(idx) => idx,
        _ => return None,
    };
    let body = dashboard_digest_body(state, digest_idx)?;
    let sections = markdown::markdown_h2_section_titles(&body);
    if sections.is_empty() {
        return None;
    }
    let next = sections
        .iter()
        .find(|title| !state.digest_folded_sections.contains(title.as_str()))
        .or_else(|| sections.first())?;
    let title = next.clone();
    state.toggle_digest_section_fold(&title);
    Some(title)
}

fn dashboard_digest_pr_refs(state: &AppState, digest_idx: usize) -> Vec<(String, u32)> {
    let fallback = state
        .config
        .repos
        .first()
        .map(String::as_str)
        .unwrap_or("unknown/repo");
    dashboard_digest_body(state, digest_idx)
        .map(|body| digest_nav::extract_pr_refs_from_digest(&body, fallback))
        .unwrap_or_default()
}

fn selected_dashboard_digest_pr(state: &AppState) -> Option<(String, u32)> {
    let digest_idx = match dashboard_row_kind_at(state, state.selected_index)? {
        DashboardRowKind::Digest(i) => i,
        _ => return None,
    };
    let refs = dashboard_digest_pr_refs(state, digest_idx);
    if refs.is_empty() {
        let fallback = state.config.repos.first().map(String::as_str)?;
        return dashboard_digest_body(state, digest_idx).and_then(|body| {
            digest_nav::pr_ref_at_source_line(&body, state.detail_scroll_line as usize, fallback)
        });
    }
    let idx = if state.dashboard_digest_pr_digest_idx == Some(digest_idx) {
        state
            .dashboard_digest_pr_index
            .min(refs.len().saturating_sub(1))
    } else {
        0
    };
    refs.get(idx).cloned()
}

fn cycle_dashboard_digest_pr(state: &mut AppState, delta: isize) -> Option<String> {
    let digest_idx = match dashboard_row_kind_at(state, state.selected_index)? {
        DashboardRowKind::Digest(i) => i,
        _ => return None,
    };
    let refs = dashboard_digest_pr_refs(state, digest_idx);
    if refs.is_empty() {
        return None;
    }
    if state.dashboard_digest_pr_digest_idx != Some(digest_idx) {
        state.dashboard_digest_pr_digest_idx = Some(digest_idx);
        state.dashboard_digest_pr_index = 0;
    }
    let len = refs.len() as isize;
    let next = (state.dashboard_digest_pr_index as isize + delta).rem_euclid(len) as usize;
    state.dashboard_digest_pr_index = next;
    let (repo, number) = &refs[next];
    Some(format!(
        "digest PR {}/{}: {repo}#{number} (Enter/p open)",
        next + 1,
        refs.len()
    ))
}

enum JumpTarget {
    Pr { repo: String, number: u32 },
}

fn jump_target_from_state(state: &AppState) -> Option<JumpTarget> {
    match state.tab {
        Tab::Dashboard => match dashboard_row_kind_at(state, state.selected_index)? {
            DashboardRowKind::Digest(_) => selected_dashboard_digest_pr(state)
                .map(|(repo, number)| JumpTarget::Pr { repo, number }),
            _ => None,
        },
        Tab::Prs => {
            let prs = state.sorted_filtered_prs();
            let p = prs.get(state.selected_index)?;
            Some(JumpTarget::Pr {
                repo: p.repo.clone(),
                number: p.number,
            })
        }
        _ => None,
    }
}

fn list_len(s: &AppState) -> usize {
    match s.tab {
        Tab::Chat => s.chat_lines.len().max(1),
        Tab::Dashboard => dashboard_rows(s).len().max(1),
        Tab::Prs => s.sorted_filtered_prs().len().max(1),
        Tab::Approvals => s.approvals.len().max(1),
        Tab::Logs => s.filtered_logs().len().max(1),
        Tab::Config => 4,
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

fn list_detail_panes(content: Rect) -> (Rect, Rect) {
    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(38), Constraint::Percentage(62)])
        .split(content);
    (panes[0], panes[1])
}

fn scroll_list_selection(s: &mut AppState, list_state: &mut ListState, delta: i32) {
    if s.tab == Tab::Chat {
        return;
    }
    let max = list_len(s).saturating_sub(1);
    let next = if delta < 0 {
        s.selected_index.saturating_sub(1)
    } else {
        (s.selected_index + 1).min(max)
    };
    if next != s.selected_index {
        s.selected_index = next;
        s.reset_detail_scroll();
        list_state.select(Some(s.selected_index));
    }
}

async fn maybe_request_pr_overview(state: &SharedState, engine: &Arc<Engine>) {
    let (repo, number, key) = {
        let s = state.read().await;
        if s.tab != Tab::Prs || !s.github_ok {
            return;
        }
        let filtered = s.sorted_filtered_prs();
        let Some(pr) = filtered.get(s.selected_index) else {
            return;
        };
        let key = AppState::pr_overview_key(&pr.repo, pr.number);
        if s.pr_overview_cache.contains_key(&key) || s.pr_overview_fetching.as_ref() == Some(&key) {
            return;
        }
        (pr.repo.clone(), pr.number, key)
    };
    {
        let mut s = state.write().await;
        if s.pr_overview_cache.contains_key(&key) {
            return;
        }
        s.pr_overview_fetching = Some(key);
    }
    engine.fetch_pr_overview(repo, number).await;
}

async fn jump_to_pr_from_context(state: &SharedState, list_state: &mut ListState) {
    let target = {
        let s = state.read().await;
        jump_target_from_state(&s)
    };
    let Some(target) = target else {
        return;
    };
    let mut s = state.write().await;
    let (ok, ok_msg, fail_msg) = match target {
        JumpTarget::Pr { repo, number } => {
            let ok = s.jump_to_pr(&repo, number);
            (
                ok,
                format!("PRs {repo}#{number}"),
                format!("PR {repo}#{number} not in store — run daily-work"),
            )
        }
    };
    if ok {
        list_state.select(Some(s.selected_index));
        s.status = ok_msg;
    } else {
        s.status = fail_msg;
    }
}

async fn handle_mouse(
    mouse: crossterm::event::MouseEvent,
    terminal: &DefaultTerminal,
    state: &SharedState,
    engine: &Arc<Engine>,
    list_state: &mut ListState,
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

    let size = terminal
        .size()
        .map_err(|e| crate::error::CoworkerError::Other(anyhow::anyhow!("terminal size: {e}")))?;
    let frame_area = Rect::new(0, 0, size.width, size.height);
    let content = ui_content_area(frame_area);
    let (list_area, detail_area) = list_detail_panes(content);
    let pos = ratatui::layout::Position::new(mouse.column, mouse.row);

    match mouse.kind {
        MouseEventKind::ScrollUp => {
            let mut s = state.write().await;
            if s.tab == Tab::Chat && (s.chat_busy || s.chat_input.is_empty()) {
                s.scroll_focused_chat_pane_line_up();
            } else if detail_area.contains(pos) && s.tab != Tab::Chat {
                s.detail_scroll_line = s.detail_scroll_line.saturating_sub(1);
            } else if list_area.contains(pos) {
                scroll_list_selection(&mut s, list_state, -1);
            }
        }
        MouseEventKind::ScrollDown => {
            let mut s = state.write().await;
            if s.tab == Tab::Chat && (s.chat_busy || s.chat_input.is_empty()) {
                s.scroll_focused_chat_pane_line_down();
            } else if detail_area.contains(pos) && s.tab != Tab::Chat {
                s.detail_scroll_line = s.detail_scroll_line.saturating_add(1);
            } else if list_area.contains(pos) {
                scroll_list_selection(&mut s, list_state, 1);
            }
        }
        MouseEventKind::Down(MouseButton::Left) => {
            let header_inner = theme::header_inner_area(frame_area);
            if header_inner.contains(ratatui::layout::Position::new(mouse.column, mouse.row)) {
                let rel_x = mouse.column.saturating_sub(header_inner.x) as usize;
                let tabs = {
                    let s = state.read().await;
                    Tab::all_for_config(&s.config)
                };
                if let Some(tab) = theme::tab_at_column(&tabs, rel_x) {
                    set_tab(state, tab, list_state).await;
                    return Ok(false);
                }
            }
            if detail_area.contains(pos) {
                let tab = {
                    let s = state.read().await;
                    s.tab
                };
                if tab != Tab::Chat {
                    let inner = detail_pane_inner(detail_area);
                    if inner.contains(pos) {
                        let line = detail_line_at_mouse(inner, mouse.row, {
                            let s = state.read().await;
                            s.detail_scroll_line
                        });
                        let mut s = state.write().await;
                        s.detail_select = Some((line, line));
                        s.detail_selecting = true;
                        return Ok(false);
                    }
                }
            }
            let content = ui_content_area(frame_area);
            let mut s = state.write().await;
            if s.tab == Tab::Chat && s.chat_context_visible {
                if let Some(focus) = focus_pane_at(content, true, mouse.column, mouse.row) {
                    s.chat_pane_focus = focus;
                }
            }
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            let tab = {
                let s = state.read().await;
                s.tab
            };
            if tab != Tab::Chat && detail_area.contains(pos) {
                let inner = detail_pane_inner(detail_area);
                if inner.contains(pos) {
                    let line = detail_line_at_mouse(inner, mouse.row, {
                        let s = state.read().await;
                        s.detail_scroll_line
                    });
                    let mut s = state.write().await;
                    if s.detail_selecting {
                        if let Some((start, _)) = s.detail_select {
                            s.detail_select = Some((start, line));
                        }
                    }
                }
            }
        }
        MouseEventKind::Up(MouseButton::Left) => {
            let (selecting, selection, tab, width) = {
                let s = state.read().await;
                (
                    s.detail_selecting,
                    s.detail_select,
                    s.tab,
                    detail_pane_inner(detail_area).width.max(1) as usize,
                )
            };
            if selecting {
                if let Some((a, b)) = selection {
                    let lo = a.min(b);
                    let hi = a.max(b);
                    let snap = {
                        let s = state.read().await;
                        copy_detail_text_from_state(&s, width, lo, hi)
                    };
                    let mut s = state.write().await;
                    s.detail_selecting = false;
                    s.detail_select = None;
                    if tab == Tab::Chat {
                        return Ok(false);
                    }
                    if let Some(text) = snap {
                        if clipboard::copy_text(&text) {
                            s.status = format!("copied {} line(s) to clipboard", hi - lo + 1);
                        } else {
                            s.status = "copy failed (install pbcopy, wl-copy, or xclip)".into();
                        }
                    }
                } else {
                    let mut s = state.write().await;
                    s.detail_selecting = false;
                    s.detail_select = None;
                }
            }
        }
        _ => {}
    }
    Ok(false)
}

fn draw_ui(frame: &mut ratatui::Frame, state: &AppState, list_state: &mut ListState) {
    let th = ThemePalette::from_tui(&state.config.tui, state.config.theme());
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

fn tab_header_label(state: &AppState, tab: Tab) -> String {
    match tab {
        Tab::Approvals if !state.approvals.is_empty() => {
            format!("3 Approvals({})", state.approvals.len())
        }
        _ => tab.label().to_string(),
    }
}

fn draw_header(frame: &mut ratatui::Frame, area: Rect, state: &AppState, th: ThemePalette) {
    // Brand mark: ✦ in accent color, followed by the product name.
    let brand: Vec<Span> = vec![
        Span::styled(
            "✦ ",
            Style::default()
                .fg(th.accent)
                .add_modifier(ratatui::style::Modifier::BOLD),
        ),
        Span::styled(
            "unistar-coworker",
            Style::default()
                .fg(th.accent)
                .add_modifier(ratatui::style::Modifier::BOLD),
        ),
        Span::raw("  "),
    ];

    let tabs_list: Vec<Span> = Tab::all_for_config(&state.config)
        .iter()
        .enumerate()
        .flat_map(|(i, t)| {
            let mut spans = Vec::new();
            if i > 0 {
                spans.push(theme::tab_separator(th));
            }
            let label = tab_header_label(state, *t);
            let mut span = theme::tab_spans(th, &label, *t == state.tab);
            if *t == Tab::Approvals && !state.approvals.is_empty() && *t != state.tab {
                span.style = span.style.fg(th.warn);
            }
            spans.push(span);
            spans
        })
        .collect();

    let block = theme::header_block(th);
    let inner = block.inner(area);
    frame.render_widget(Paragraph::new("").block(block), area);
    let mut all_spans = brand;
    all_spans.extend(tabs_list);
    frame.render_widget(
        Paragraph::new(Line::from(all_spans)).style(Style::default().bg(th.surface)),
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
                } else if state.chat_pane_focus_is_context() {
                    format!(
                        "click/←/→: focus  ↑/↓: scroll  j/k: msgs  \\: ctx  End: latest{approval_hint}"
                    )
                } else {
                    format!(
                        "click/←/→: focus  ↑/↓: scroll  o: expand tool (input empty)  j/k: scroll  \\: ctx  End: latest{approval_hint}"
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
            "r: daily  R: radar  v: radar  m: PRs  c: chat  o: refresh overview  Enter/p: open PR  n/N: cycle PR  z: fold list  Z: fold digest  {/}: detail scroll".into()
        }
        Tab::Prs => return frame.render_widget(
            Paragraph::new(theme::hint_bar(
                th,
                &format!(
                    "filter: {} (/)  sort: {} (s)  t: triage  o: refresh  p: focus PR  j/k scroll  q: quit",
                    state.pr_filter.label(),
                    state.pr_sort.label()
                ),
            ))
            .style(Style::default().bg(th.title_bg)),
            area,
        ),
        Tab::Approvals => "y: approve (runs tool)  n: deny  q: quit".into(),
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
        _ => "j/k: scroll  {/}: detail  drag detail: copy  Config: R probe  Tab: next  q: quit".into(),
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
        Tab::Dashboard => dashboard_rows(state)
            .into_iter()
            .map(|row| {
                let style = match row.kind {
                    DashboardRowKind::SectionHeader(_) => {
                        Style::default().fg(th.accent).add_modifier(Modifier::BOLD)
                    }
                    _ => Style::default().fg(th.text),
                };
                ListItem::new(Line::from(Span::styled(row.label, style)))
            })
            .collect(),
        Tab::Prs => {
            let filtered = state.sorted_filtered_prs();
            if filtered.is_empty() {
                vec![ListItem::new(format!(
                    "No PRs ({}) — run daily-work",
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
                            if p.triage_note.is_some() {
                                Span::styled("◆ ", Style::default().fg(th.accent_dim))
                            } else {
                                Span::raw("")
                            },
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
        Tab::Config => {
            let mut items = vec![
                ListItem::new(format!("config: {}", state.config_path)),
                ListItem::new(format!("repos: {}", state.config.repos.join(", "))),
                ListItem::new(format!("storage: {:?}", state.config.storage.backend)),
                ListItem::new(format!("llm: {}", state.config.llm.model)),
                ListItem::new(format!(
                    "github: {}",
                    context_panel::format_probe_latency(state.github_ok, state.github_latency_ms)
                )),
                ListItem::new(format!(
                    "llm probe: {}",
                    context_panel::format_probe_latency(state.llm_ok, state.llm_latency_ms)
                )),
                ListItem::new(format!("theme: {:?}", state.config.theme())),
            ];
            for server in &state.mcp_servers {
                let label = if server.connected {
                    format!(
                        "mcp[{}]: ok ({} tools, {}ms)",
                        server.id,
                        server.tool_count,
                        server.last_rpc_ms.unwrap_or(0)
                    )
                } else if let Some(err) = &server.last_error {
                    format!("mcp[{}]: err ({err})", server.id)
                } else {
                    format!("mcp[{}]: offline", server.id)
                };
                items.push(ListItem::new(label));
            }
            items
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
        state.detail_select,
    );
}

struct DetailBody {
    text: String,
    markdown: bool,
}

fn config_connectivity_detail(state: &AppState) -> String {
    const BAR_W: usize = 24;
    let github_status =
        context_panel::format_probe_latency(state.github_ok, state.github_latency_ms);
    let llm_status = context_panel::format_probe_latency(state.llm_ok, state.llm_latency_ms);
    let github_bar = state
        .github_ok
        .then_some(state.github_latency_ms)
        .flatten()
        .map(|ms| format!("\n\n`{}`", context_panel::latency_bar_ms(ms, BAR_W)))
        .unwrap_or_default();
    let llm_bar = state
        .llm_ok
        .then_some(state.llm_latency_ms)
        .flatten()
        .map(|ms| format!("\n\n`{}`", context_panel::latency_bar_ms(ms, BAR_W)))
        .unwrap_or_default();
    format!(
        "## Connectivity\n\n\
        | Service | Endpoint | Status |\n|---|---|---|\n\
        | GitHub (`gh`) | {} | {} |\n\
        | LLM | {} | {} |\n\n\
        ### Latency (engine start)\n\n\
        **GitHub** — {github_status}{github_bar}\n\n\
        **LLM** — {llm_status}{llm_bar}\n\n\
        _Bar scale: 0–2000ms_\n\n\
        Press **R** to re-probe GitHub and LLM latency.\n\n\
        ## Enabled workflows\n\n{}",
        state.config.github.gh_command,
        if state.github_ok { "ok" } else { "offline" },
        state.config.llm.base_url,
        if state.llm_ok { "ok" } else { "offline" },
        state
            .config
            .workflows
            .iter()
            .filter(|(_, w)| w.enabled)
            .map(|(k, _)| format!("- `{k}`"))
            .collect::<Vec<_>>()
            .join("\n")
    )
}

fn detail_body(state: &AppState) -> (String, bool) {
    let view = match state.tab {
        Tab::Chat => DetailBody {
            text: String::new(),
            markdown: false,
        },
        Tab::Dashboard => match dashboard_row_kind_at(state, state.selected_index) {
            Some(DashboardRowKind::Digest(idx)) => {
                let raw = dashboard_digest_body(state, idx)
                    .unwrap_or_else(|| "Press r to run daily-work.".into());
                let has_md = idx == 0
                    || state
                        .digest_history
                        .get(idx)
                        .is_some_and(|meta| state.digest_bodies.contains_key(&meta.date));
                DetailBody {
                    text: if has_md {
                        markdown::filter_folded_markdown_sections(
                            &raw,
                            &state.digest_folded_sections,
                            "Z",
                        )
                    } else {
                        raw
                    },
                    markdown: has_md,
                }
            }
            Some(DashboardRowKind::SectionHeader(_)) => DetailBody {
                text: "Select an item in this section, or press **z** to fold/unfold.".into(),
                markdown: true,
            },
            _ => DetailBody {
                text: "Press **r** to run daily-work.".into(),
                markdown: true,
            },
        },
        Tab::Prs => {
            if let Some(overview) = state.selected_pr_overview() {
                let triage = state
                    .sorted_filtered_prs()
                    .get(state.selected_index)
                    .and_then(|p| p.triage_note.as_deref())
                    .map(|n| format!("\n\n## Triage note\n\n{n}"))
                    .unwrap_or_default();
                DetailBody {
                    text: format!("{overview}{triage}"),
                    markdown: true,
                }
            } else if state.selected_pr_overview_loading() {
                DetailBody {
                    text: "_Loading `pr_get_overview`…_".into(),
                    markdown: true,
                }
            } else {
                DetailBody {
                    text: state
                        .sorted_filtered_prs()
                        .get(state.selected_index)
                        .map(|p| {
                            format!(
                                "# #{} {}\n\n\
                                | Field | Value |\n|---|---|\n\
                                | Author | @{} |\n\
                                | Repo | {} |\n\
                                | CI | {} |\n\
                                | Review | {} |\n\n\
                                ## Triage\n\n{}",
                                p.number,
                                p.title,
                                p.author,
                                p.repo,
                                p.ci_summary,
                                p.review_summary,
                                p.triage_note.as_deref().unwrap_or("_No triage yet._")
                            )
                        })
                        .unwrap_or_else(|| "Select a PR".into()),
                    markdown: true,
                }
            }
        }
        Tab::Approvals => DetailBody {
            text: state
                .selected_approval()
                .map(format_approval_detail)
                .unwrap_or_else(|| "Select an approval".into()),
            markdown: true,
        },
        Tab::Logs => {
            let logs: Vec<_> = state.filtered_logs().into_iter().rev().collect();
            DetailBody {
                text: logs
                    .get(state.selected_index)
                    .map(|l| {
                        format!(
                            "## [{}] {}\n\n```\n{}\n```",
                            l.level,
                            l.ts.format("%Y-%m-%d %H:%M:%S"),
                            l.message
                        )
                    })
                    .unwrap_or_else(|| format!("No logs ({})", state.log_filter.label())),
                markdown: true,
            }
        }
        Tab::Config => DetailBody {
            text: config_connectivity_detail(state),
            markdown: true,
        },
    };
    (view.text, view.markdown)
}

fn format_approval_detail(a: &crate::store::Approval) -> String {
    let mut md = format!(
        "# {:?} approval\n\n\
        | Field | Value |\n|---|---|\n\
        | ID | `{}` |\n\
        | Status | {:?} |\n\
        | Repo | {} |\n\
        | Created | {} |\n",
        a.kind,
        a.id,
        a.status,
        a.repo,
        a.created_at.format("%Y-%m-%d %H:%M:%S")
    );
    if let Some(at) = a.decided_at {
        md.push_str(&format!(
            "| Decided | {} |\n",
            at.format("%Y-%m-%d %H:%M:%S")
        ));
    }
    if let Some(n) = a.pr_number {
        md.push_str(&format!("| PR | #{n} |\n"));
    }
    if let Some(run) = a.run_id {
        md.push_str(&format!("| Run | {run} |\n"));
    }
    if let Some(ref branch) = a.target_branch {
        md.push_str(&format!("| Branch | {branch} |\n"));
    }
    if let Some(id) = a.incident_id {
        md.push_str(&format!("| Incident | {id} |\n"));
    }
    md.push_str("\n## Description\n\n");
    md.push_str(&a.description);
    if let Some(ref body) = a.comment_body {
        md.push_str("\n\n## Comment body\n\n");
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(body) {
            if let Ok(pretty) = serde_json::to_string_pretty(&value) {
                md.push_str("```json\n");
                md.push_str(&pretty);
                md.push_str("\n```\n");
            } else {
                md.push_str(body);
            }
        } else {
            md.push_str(body);
        }
    }
    if let Some(issue) = a.issue_number {
        md.push_str(&format!("\n\n## Issue\n\n#{issue}\n"));
    }
    if let Some(ref label) = a.label {
        md.push_str(&format!("\n\n## Label\n\n`{label}`\n"));
    }
    if let Ok(pretty) = serde_json::to_string_pretty(&approval_tool_payload(a)) {
        md.push_str("\n\n## Tool payload\n\n```json\n");
        md.push_str(&pretty);
        md.push_str("\n```\n");
    }
    md
}

fn approval_tool_payload(a: &crate::store::Approval) -> serde_json::Value {
    use crate::store::model::ApprovalKind;
    match a.kind {
        ApprovalKind::RerunFlaky => serde_json::json!({
            "action": "ci_rerun",
            "repo": a.repo,
            "pr_number": a.pr_number,
            "run_id": a.run_id,
            "incident_id": a.incident_id,
        }),
        ApprovalKind::Backport => serde_json::json!({
            "action": "backport",
            "repo": a.repo,
            "pr_number": a.pr_number,
            "target_branch": a.target_branch,
        }),
        ApprovalKind::PostComment => serde_json::json!({
            "action": "pr_post_comment",
            "repo": a.repo,
            "pr_number": a.pr_number,
            "body": a.comment_body,
        }),
        ApprovalKind::IssueAddLabel => serde_json::json!({
            "action": "issue_add_label",
            "repo": a.repo,
            "issue_number": a.issue_number,
            "label": a.label,
        }),
        ApprovalKind::WriteFile | ApprovalKind::EditFile => serde_json::json!({
            "action": "file_mutation",
            "workspace": a.repo,
            "kind": format!("{:?}", a.kind),
            "args": a.comment_body,
        }),
        ApprovalKind::BashRun => serde_json::json!({
            "action": "bash_run",
            "workspace": a.repo,
            "args": a.comment_body,
        }),
        ApprovalKind::PythonRun => serde_json::json!({
            "action": "python_run",
            "workspace": a.repo,
            "args": a.comment_body,
        }),
        ApprovalKind::McpTool => serde_json::json!({
            "action": "mcp_tool",
            "workspace": a.repo,
            "args": a.comment_body,
        }),
    }
}

fn detail_pane_inner(area: Rect) -> Rect {
    theme::detail_block(ThemePalette::dark()).inner(area)
}

fn detail_line_at_mouse(inner: Rect, mouse_row: u16, scroll: u16) -> u16 {
    scroll + mouse_row.saturating_sub(inner.y)
}

fn detail_render_lines(
    th: ThemePalette,
    body: &str,
    render_markdown: bool,
    width: usize,
) -> Vec<Line<'static>> {
    let base = Style::default().fg(th.text);
    if render_markdown {
        let key = detail_body_cache_key(body, width.min(u16::MAX as usize) as u16);
        cached_detail_markdown_lines(th, body, width, key)
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
    }
}

fn line_plain(line: &Line) -> String {
    line.spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect::<String>()
}

fn copy_detail_text_from_state(
    state: &AppState,
    width: usize,
    sel_lo: u16,
    sel_hi: u16,
) -> Option<String> {
    let th = ThemePalette::from_tui(&state.config.tui, state.config.theme());
    let (body, render_markdown) = detail_body(state);
    let lines = detail_render_lines(th, &body, render_markdown, width);
    if lines.is_empty() {
        return None;
    }
    let lo = sel_lo.min(sel_hi) as usize;
    let hi = sel_lo.max(sel_hi).min(lines.len().saturating_sub(1) as u16) as usize;
    let text = lines[lo..=hi]
        .iter()
        .map(line_plain)
        .collect::<Vec<_>>()
        .join("\n");
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn apply_detail_selection_highlight(
    lines: &mut [Line<'static>],
    th: ThemePalette,
    select: Option<(u16, u16)>,
) {
    let Some((a, b)) = select else {
        return;
    };
    let lo = a.min(b) as usize;
    let hi = a.max(b) as usize;
    for (i, line) in lines.iter_mut().enumerate() {
        if (lo..=hi).contains(&i) {
            for span in &mut line.spans {
                span.style = span.style.bg(th.tab_active_bg);
            }
        }
    }
}

fn draw_detail_pane(
    frame: &mut ratatui::Frame,
    area: Rect,
    th: ThemePalette,
    body: &str,
    render_markdown: bool,
    scroll_line: u16,
    detail_select: Option<(u16, u16)>,
) {
    let block = theme::detail_block(th);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let base = Style::default().fg(th.text);
    let mut lines: Vec<Line> = if render_markdown {
        let key = detail_body_cache_key(body, inner.width.max(1));
        cached_detail_markdown_lines(th, body, inner.width.max(1) as usize, key)
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
    if lines.is_empty() {
        lines.push(Line::from(""));
    }
    apply_detail_selection_highlight(&mut lines, th, detail_select);
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
    let phase_note = state
        .chat_turn_phase()
        .map(|p| format!(" │ phase: {p}"))
        .unwrap_or_default();
    let workflow_note = state
        .engine_workflow_id
        .as_ref()
        .filter(|_| state.engine_busy)
        .map(|id| format!(" │ {id}"))
        .unwrap_or_default();
    let auto_approve_note = if state.config.chat.auto_approve_mutations {
        " │ auto-approve ON"
    } else {
        ""
    };
    let mut line = theme::status_line(
        th,
        busy,
        &state.status,
        state.github_ok,
        state.llm_ok,
        auto_approve_note,
    );
    line.spans.extend(context_status_spans(th, state));
    line.spans.extend(store_status_spans(th, state));
    if !workflow_note.is_empty() {
        line.spans.push(Span::styled(
            workflow_note,
            Style::default().fg(th.accent).bg(th.surface),
        ));
    }
    if !phase_note.is_empty() {
        line.spans.push(Span::styled(
            phase_note,
            Style::default().fg(th.muted).bg(th.surface),
        ));
    }
    line.spans
        .push(Span::styled(" ", Style::default().bg(th.surface)));
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

#[cfg(test)]
mod detail_tests {
    use super::*;
    use crate::store::model::{Approval, ApprovalKind, ApprovalStatus};
    use chrono::Utc;
    use uuid::Uuid;

    #[test]
    fn approval_detail_includes_tool_payload_json_fence() {
        let approval = Approval {
            id: Uuid::new_v4(),
            kind: ApprovalKind::RerunFlaky,
            repo: "acme/widget".into(),
            pr_number: Some(42),
            run_id: Some(99),
            target_branch: None,
            incident_id: None,
            description: "rerun flaky job".into(),
            status: ApprovalStatus::Pending,
            created_at: Utc::now(),
            decided_at: None,
            comment_body: None,
            issue_number: None,
            label: None,
        };
        let md = format_approval_detail(&approval);
        assert!(md.contains("## Tool payload"));
        assert!(md.contains("```json"));
        assert!(md.contains("\"action\": \"ci_rerun\""));
        assert!(md.contains("\"pr_number\": 42"));
    }

    #[test]
    fn approval_tool_payload_maps_post_comment() {
        let approval = Approval {
            id: Uuid::new_v4(),
            kind: ApprovalKind::PostComment,
            repo: "acme/widget".into(),
            pr_number: Some(1),
            run_id: None,
            target_branch: None,
            incident_id: None,
            description: "comment".into(),
            status: ApprovalStatus::Pending,
            created_at: Utc::now(),
            decided_at: None,
            comment_body: Some("hello".into()),
            issue_number: None,
            label: None,
        };
        let payload = approval_tool_payload(&approval);
        assert_eq!(payload["action"], "pr_post_comment");
        assert_eq!(payload["body"], "hello");
    }
}
