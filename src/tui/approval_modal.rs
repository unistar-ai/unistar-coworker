use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;
use uuid::Uuid;

use crate::app::{spawn_approval_decision, AppState, ApprovalDialogChoice, SharedState};
use crate::engine::Engine;
use crate::tui::theme::ThemePalette;

const MODAL_WIDTH_PCT: u16 = 54;
const MODAL_HEIGHT_PCT: u16 = 32;

struct ModalLayout {
    modal_area: Rect,
    approve_button: Rect,
    deny_button: Rect,
}

pub fn draw_approval_modal(frame: &mut Frame, state: &AppState, th: ThemePalette) {
    let Some(dialog) = &state.approval_dialog else {
        return;
    };

    let screen = frame.area();
    let layout = modal_layout(screen);

    // Dim the UI behind the dialog so stray glyphs cannot bleed through.
    frame.render_widget(
        Block::default().style(Style::default().bg(scrim_color(th))),
        screen,
    );

    frame.render_widget(Clear, layout.modal_area);

    let title = if dialog.deciding {
        " ⏳ Processing "
    } else {
        " ⚠ Approval required "
    };

    let block = modal_block(th, title);
    let inner = block.inner(layout.modal_area);
    frame.render_widget(block, layout.modal_area);
    fill_rect(frame, inner, th.panel);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(2),
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .split(inner);

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Tool  ", Style::default().fg(th.muted)),
            Span::styled(
                &dialog.tool_name,
                Style::default().fg(th.warn).add_modifier(Modifier::BOLD),
            ),
        ])),
        chunks[0],
    );

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Action  ", Style::default().fg(th.muted)),
            Span::styled("mutating — needs your OK", Style::default().fg(th.text)),
        ])),
        chunks[1],
    );

    let detail_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(Style::default().fg(th.border))
        .style(Style::default().bg(th.surface));
    let detail_inner = detail_block.inner(chunks[2]);
    frame.render_widget(detail_block, chunks[2]);
    fill_rect(frame, detail_inner, th.surface);
    frame.render_widget(
        Paragraph::new(dialog.description.as_str())
            .wrap(Wrap { trim: true })
            .style(Style::default().fg(th.text)),
        detail_inner,
    );

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("ID  ", Style::default().fg(th.muted)),
            Span::styled(short_uuid(&dialog.id), Style::default().fg(th.muted)),
        ])),
        chunks[3],
    );

    if dialog.deciding {
        fill_rect(frame, chunks[4], th.panel);
        frame.render_widget(
            Paragraph::new("Sending decision…")
                .alignment(Alignment::Center)
                .style(
                    Style::default()
                        .fg(th.accent)
                        .add_modifier(Modifier::ITALIC),
                ),
            chunks[4],
        );
    } else {
        let selected = dialog.choice;
        let armed = dialog.approve_armed();
        let approve_label = if armed {
            " ✓ Approve ".to_string()
        } else {
            let ms = dialog.approve_arm_ms_remaining().max(1);
            format!(" ✓ Approve ({ms}ms) ")
        };
        draw_modal_button(
            frame,
            layout.approve_button,
            &approve_label,
            selected == ApprovalDialogChoice::Approve,
            if armed { th.ok } else { th.muted },
            th,
        );
        draw_modal_button(
            frame,
            layout.deny_button,
            " ✗ Deny ",
            selected == ApprovalDialogChoice::Deny,
            th.err,
            th,
        );
    }

    if !dialog.deciding {
        let arm_note = if dialog.approve_armed() {
            "click · ←/→ · Tab · Enter/y approve · n/Esc deny"
        } else {
            "approve arms shortly — deny is immediate"
        };
        frame.render_widget(
            Paragraph::new(arm_note)
                .alignment(Alignment::Center)
                .style(Style::default().fg(th.muted)),
            chunks[5],
        );
    }
}

fn modal_block<'a>(th: ThemePalette, title: &'a str) -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(th.border_active))
        .style(Style::default().bg(th.panel))
        .title(Span::styled(
            title,
            Style::default()
                .fg(th.warn)
                .bg(th.panel)
                .add_modifier(Modifier::BOLD),
        ))
}

fn fill_rect(frame: &mut Frame, area: Rect, bg: ratatui::style::Color) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    frame.render_widget(Block::default().style(Style::default().bg(bg)), area);
}

fn scrim_color(th: ThemePalette) -> ratatui::style::Color {
    // Slightly darker than the main background to suggest a modal overlay.
    match th.bg {
        ratatui::style::Color::Rgb(r, g, b) => ratatui::style::Color::Rgb(
            r.saturating_sub(6),
            g.saturating_sub(6),
            b.saturating_sub(6),
        ),
        other => other,
    }
}

fn short_uuid(id: &Uuid) -> String {
    let s = id.to_string();
    format!("{}…", &s[..8.min(s.len())])
}

fn draw_modal_button(
    frame: &mut Frame,
    area: Rect,
    label: &str,
    selected: bool,
    accent: ratatui::style::Color,
    th: ThemePalette,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    fill_rect(frame, area, th.panel);
    let border = if selected { accent } else { th.border };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border))
        .style(Style::default().bg(if selected { th.surface } else { th.panel }));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    fill_rect(frame, inner, if selected { th.surface } else { th.panel });
    frame.render_widget(
        Paragraph::new(label).alignment(Alignment::Center).style(
            Style::default()
                .fg(if selected { accent } else { th.text })
                .add_modifier(if selected {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
        ),
        inner,
    );
}

pub async fn handle_approval_modal_mouse(
    mouse: MouseEvent,
    frame_area: Rect,
    state: &SharedState,
    engine: &Arc<Engine>,
) {
    if !matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
        return;
    }
    let id = {
        let s = state.read().await;
        if s.approval_decision_busy() {
            return;
        }
        let Some(dialog) = s.approval_dialog.as_ref() else {
            return;
        };
        if dialog.deciding {
            return;
        }
        dialog.id
    };
    let layout = modal_layout(frame_area);
    let pos = Position::new(mouse.column, mouse.row);
    if layout.approve_button.contains(pos) {
        let armed = {
            let s = state.read().await;
            s.approval_dialog
                .as_ref()
                .is_some_and(|d| d.id == id && d.approve_armed())
        };
        if armed {
            spawn_approval_decision(state, engine, id, true).await;
        }
    } else if layout.deny_button.contains(pos) {
        spawn_approval_decision(state, engine, id, false).await;
    }
}

pub async fn handle_approval_modal_key(
    key: KeyEvent,
    state: &SharedState,
    engine: &Arc<Engine>,
) -> bool {
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return true;
    }

    let action = {
        let s = state.read().await;
        let Some(dialog) = s.approval_dialog.as_ref() else {
            return false;
        };
        if dialog.deciding || s.approval_decision_busy() {
            return false;
        }
        match key.code {
            KeyCode::Left | KeyCode::Right | KeyCode::Tab => Some(ModalAction::Toggle),
            KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => {
                let approve = dialog.choice == ApprovalDialogChoice::Approve;
                if approve && !dialog.approve_armed() {
                    Some(ModalAction::Ignore)
                } else {
                    Some(ModalAction::Decide {
                        id: dialog.id,
                        approve,
                    })
                }
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => Some(ModalAction::Decide {
                id: dialog.id,
                approve: false,
            }),
            _ => Some(ModalAction::Ignore),
        }
    };

    match action {
        None => false,
        Some(ModalAction::Toggle) => {
            let mut s = state.write().await;
            s.toggle_approval_dialog_choice();
            false
        }
        Some(ModalAction::Ignore) => false,
        Some(ModalAction::Decide { id, approve }) => {
            spawn_approval_decision(state, engine, id, approve).await;
            false
        }
    }
}

enum ModalAction {
    Toggle,
    Decide { id: Uuid, approve: bool },
    Ignore,
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

/// Shared geometry for draw + mouse hit-testing. Uses the same block metrics as `draw_approval_modal`.
fn modal_layout(frame_area: Rect) -> ModalLayout {
    let modal_area = centered_rect(MODAL_WIDTH_PCT, MODAL_HEIGHT_PCT, frame_area);
    let inner = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(" ⚠ Approval required ")
        .inner(modal_area);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(2),
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .split(inner);
    let buttons = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(50),
            Constraint::Length(1),
            Constraint::Percentage(50),
        ])
        .split(chunks[4]);
    ModalLayout {
        modal_area,
        approve_button: buttons[0],
        deny_button: buttons[2],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_uuid_truncates() {
        let id = Uuid::parse_str("79ac55d9-e785-4949-b282-eb90270756f2").unwrap();
        assert_eq!(short_uuid(&id), "79ac55d9…");
    }

    #[test]
    fn modal_button_rects_fit_inside_modal() {
        let frame = Rect::new(0, 0, 120, 40);
        let layout = modal_layout(frame);
        assert!(layout.modal_area.contains(Position::new(
            layout.approve_button.x,
            layout.approve_button.y
        )));
        assert!(layout
            .modal_area
            .contains(Position::new(layout.deny_button.x, layout.deny_button.y)));
        assert!(layout.approve_button.width > 0);
        assert!(layout.deny_button.width > 0);
    }

    #[test]
    fn approval_not_armed_immediately() {
        use crate::app::{ApprovalDialog, ApprovalDialogChoice};
        use std::time::Instant;
        use uuid::Uuid;

        let dialog = ApprovalDialog {
            id: Uuid::new_v4(),
            tool_name: "ci_rerun".into(),
            description: "rerun".into(),
            choice: ApprovalDialogChoice::Approve,
            deciding: false,
            opened_at: Instant::now(),
        };
        assert!(!dialog.approve_armed());
        assert!(dialog.approve_arm_ms_remaining() > 0);
    }
}
