use std::sync::Arc;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::DefaultTerminal;
use tokio::sync::broadcast;
use unicode_width::UnicodeWidthStr;

use crate::app::{AppEvent, AppState, SharedState, Tab};
use crate::engine::Engine;
use crate::error::Result;
use crate::store::Store;

pub async fn run(
    terminal: &mut DefaultTerminal,
    state: SharedState,
    engine: Arc<Engine>,
    _store: Arc<dyn Store>,
    mut events_rx: broadcast::Receiver<AppEvent>,
) -> Result<()> {
    let mut list_state = ListState::default();
    list_state.select(Some(0));

    loop {
        while let Ok(ev) = events_rx.try_recv() {
            apply_event(&state, ev).await;
        }

        {
            let s = state.read().await;
            terminal.draw(|frame| draw_ui(frame, &s, &mut list_state))?;
        }

        if event::poll(Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                if handle_key(key, &state, &engine, &mut list_state).await? {
                    break;
                }
            }
        }
    }
    Ok(())
}

async fn apply_event(state: &SharedState, ev: AppEvent) {
    let mut s = state.write().await;
    match ev {
        AppEvent::StoreUpdated => s.status = "store updated".into(),
        AppEvent::DigestReady(d) => {
            s.latest_digest = Some(d);
            s.status = "digest ready".into();
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
            s.push_log(
                "info",
                format!("{workflow_id} finished: {message}"),
            );
        }
        AppEvent::StatusMessage(m) => {
            s.status = m.clone();
            s.push_log("info", m);
        }
    }
}

async fn handle_key(
    key: KeyEvent,
    state: &SharedState,
    engine: &Arc<Engine>,
    list_state: &mut ListState,
) -> Result<bool> {
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return Ok(true);
    }

    match key.code {
        KeyCode::Char('q') => return Ok(true),
        KeyCode::Char('1') => set_tab(state, Tab::Dashboard, list_state).await,
        KeyCode::Char('2') => set_tab(state, Tab::Prs, list_state).await,
        KeyCode::Char('3') => set_tab(state, Tab::Approvals, list_state).await,
        KeyCode::Char('4') => set_tab(state, Tab::Logs, list_state).await,
        KeyCode::Char('5') => set_tab(state, Tab::Config, list_state).await,
        KeyCode::Char('6') => set_tab(state, Tab::Flaky, list_state).await,
        KeyCode::Tab => {
            let mut s = state.write().await;
            s.tab = s.tab.next();
            s.selected_index = 0;
            list_state.select(Some(0));
        }
        KeyCode::BackTab => {
            let mut s = state.write().await;
            s.tab = s.tab.prev();
            s.selected_index = 0;
            list_state.select(Some(0));
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
        KeyCode::Char('y') => {
            let id = {
                let s = state.read().await;
                if s.tab == Tab::Approvals {
                    s.approvals.get(s.selected_index).map(|a| a.id)
                } else {
                    None
                }
            };
            if let Some(id) = id {
                let engine = Arc::clone(engine);
                let state = state.clone();
                tokio::spawn(async move {
                    match engine.decide_approval(&id, true).await {
                        Ok(msg) => {
                            let mut s = state.write().await;
                            s.push_log("info", format!("approved: {msg}"));
                            s.status = msg;
                        }
                        Err(e) => {
                            let mut s = state.write().await;
                            s.push_log("error", format!("approval failed: {e}"));
                            s.status = format!("error: {e}");
                        }
                    }
                });
            }
        }
        KeyCode::Char('n') => {
            let id = {
                let s = state.read().await;
                if s.tab == Tab::Approvals {
                    s.approvals.get(s.selected_index).map(|a| a.id)
                } else {
                    None
                }
            };
            if let Some(id) = id {
                let engine = Arc::clone(engine);
                let state = state.clone();
                tokio::spawn(async move {
                    match engine.decide_approval(&id, false).await {
                        Ok(msg) => {
                            let mut s = state.write().await;
                            s.push_log("info", msg);
                        }
                        Err(e) => {
                            let mut s = state.write().await;
                            s.push_log("error", format!("deny failed: {e}"));
                        }
                    }
                });
            }
        }
        KeyCode::Up | KeyCode::Char('k') => {
            let mut s = state.write().await;
            if s.selected_index > 0 {
                s.selected_index -= 1;
                list_state.select(Some(s.selected_index));
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            let mut s = state.write().await;
            let max = list_len(&s).saturating_sub(1);
            if s.selected_index < max {
                s.selected_index += 1;
                list_state.select(Some(s.selected_index));
            }
        }
        _ => {}
    }
    Ok(false)
}

async fn set_tab(state: &SharedState, tab: Tab, list_state: &mut ListState) {
    let mut s = state.write().await;
    s.tab = tab;
    s.selected_index = 0;
    list_state.select(Some(0));
}

fn list_len(s: &AppState) -> usize {
    match s.tab {
        Tab::Dashboard => s.digest_history.len().max(1),
        Tab::Prs => s.prs.len().max(1),
        Tab::Approvals => s.approvals.len().max(1),
        Tab::Logs => s.logs.len().max(1),
        Tab::Config => 4,
        Tab::Flaky => s.flaky_tests.len().max(1),
    }
}

fn draw_ui(frame: &mut ratatui::Frame, state: &AppState, list_state: &mut ListState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(frame.area());

    draw_header(frame, chunks[0], state);
    draw_hints(frame, chunks[1], state);

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(chunks[2]);

    draw_list(frame, body[0], state, list_state);
    draw_detail(frame, body[1], state);
    draw_status(frame, chunks[3], state);
}

fn draw_header(frame: &mut ratatui::Frame, area: Rect, state: &AppState) {
    let tabs: Line = Line::from(
        Tab::ALL
            .iter()
            .map(|t| {
                let style = if *t == state.tab {
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                Span::styled(format!(" {} ", t.label()), style)
            })
            .collect::<Vec<_>>(),
    );
    let block = Block::default().borders(Borders::ALL).title(" unistar-coworker ");
    frame.render_widget(Paragraph::new(tabs).block(block), area);
}

fn draw_hints(frame: &mut ratatui::Frame, area: Rect, state: &AppState) {
    let hint = match state.tab {
        Tab::Dashboard => "r: daily-work  R: release-duty  j/k: scroll  Tab: next  q: quit",
        Tab::Approvals => "y: approve (runs MCP)  n: deny  q: quit",
        _ => "j/k: scroll  Tab: next  q: quit",
    };
    frame.render_widget(Paragraph::new(hint).style(Style::default().dim()), area);
}

fn draw_list(frame: &mut ratatui::Frame, area: Rect, state: &AppState, list_state: &mut ListState) {
    let items: Vec<ListItem> = match state.tab {
        Tab::Dashboard => {
            if state.digest_history.is_empty() {
                vec![ListItem::new("No digest — press r")]
            } else {
                state
                    .digest_history
                    .iter()
                    .map(|d| {
                        ListItem::new(format!(
                            "{} — attention:{} flaky:{} ok:{} ({})",
                            d.date,
                            d.summary.needs_attention,
                            d.summary.flaky_candidates,
                            d.summary.ignorable,
                            d.summary.duration_label()
                        ))
                    })
                    .collect()
            }
        }
        Tab::Prs => {
            if state.prs.is_empty() {
                vec![ListItem::new("No PRs — run daily-work")]
            } else {
                state
                    .prs
                    .iter()
                    .map(|p| {
                        ListItem::new(format!(
                            "#{} {} [{}]",
                            p.number,
                            trunc(&p.title, 36),
                            p.repo
                        ))
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
        Tab::Logs => state
            .logs
            .iter()
            .rev()
            .take(80)
            .map(|l| ListItem::new(format!("[{}] {}", l.level, trunc(&l.message, 50))))
            .collect(),
        Tab::Config => vec![
            ListItem::new(format!("config: {}", state.config_path)),
            ListItem::new(format!("repos: {}", state.config.repos.join(", "))),
            ListItem::new(format!("storage: {:?}", state.config.storage.backend)),
            ListItem::new(format!("llm: {}", state.config.llm.model)),
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
    };

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(" List "))
        .highlight_style(Style::default().bg(Color::DarkGray));
    frame.render_stateful_widget(list, area, list_state);
}

fn draw_detail(frame: &mut ratatui::Frame, area: Rect, state: &AppState) {
    let text = match state.tab {
        Tab::Dashboard => {
            if state.selected_index == 0 {
                state
                    .latest_digest
                    .as_ref()
                    .map(|d| d.body_md.clone())
                    .unwrap_or_else(|| "Press r to run daily-work.".into())
            } else if let Some(meta) = state.selected_digest() {
                format!(
                    "Digest {}\nattention: {}  flaky: {}  ok: {}\nrun time: {}\n\n(full body only for latest run)",
                    meta.date,
                    meta.summary.needs_attention,
                    meta.summary.flaky_candidates,
                    meta.summary.ignorable,
                    meta.summary.duration_label()
                )
            } else {
                "Press r to run daily-work.".into()
            }
        }
        Tab::Prs => state
            .selected_pr()
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
        Tab::Approvals => state
            .selected_approval()
            .map(|a| {
                format!(
                    "{:?}\n{}\nrepo: {}\npr: {:?}\nrun: {:?}\nbranch: {:?}",
                    a.kind,
                    a.description,
                    a.repo,
                    a.pr_number,
                    a.run_id,
                    a.target_branch
                )
            })
            .unwrap_or_else(|| "Select an approval".into()),
        Tab::Logs => state
            .logs
            .iter()
            .rev()
            .take(40)
            .map(|l| format!("[{}] {}", l.ts.format("%H:%M:%S"), l.message))
            .collect::<Vec<_>>()
            .join("\n"),
        Tab::Config => format!(
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
        Tab::Flaky => state
            .flaky_tests
            .get(state.selected_index)
            .map(|t| {
                format!(
                    "fingerprint: {}\nrepo: {}\nworkflow: {}\ncount: {}\nrerun: {}/{}\nlast: {}",
                    t.fingerprint,
                    t.repo,
                    t.workflow,
                    t.incident_count,
                    t.rerun_successes,
                    t.rerun_attempts,
                    t.last_seen
                )
            })
            .unwrap_or_else(|| "Select a flaky entry".into()),
    };

    frame.render_widget(
        Paragraph::new(text)
            .block(Block::default().borders(Borders::ALL).title(" Detail "))
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn draw_status(frame: &mut ratatui::Frame, area: Rect, state: &AppState) {
    let busy = if state.engine_busy { "busy" } else { "idle" };
    let line = format!(
        "status: {busy} | {} | mcp: {} | llm: {}",
        state.status,
        if state.mcp_ok { "ok" } else { "off" },
        if state.llm_ok { "ok" } else { "off" }
    );
    frame.render_widget(Paragraph::new(line), area);
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
