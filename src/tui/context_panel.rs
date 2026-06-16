use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, Wrap};

use crate::agent::chat_loop::ContextSnapshot;
use crate::app::AppState;
use crate::tui::markdown::markdown_to_lines_in_width;
use crate::tui::scroll::paragraph_scrollbar_state;
use crate::tui::theme::{self, ThemePalette};

const CONTEXT_SCROLL_PAGE: u16 = 8;

pub fn scroll_context_page_up(state: &mut AppState) {
    state.chat_context_scroll_from_bottom = state
        .chat_context_scroll_from_bottom
        .saturating_add(CONTEXT_SCROLL_PAGE);
}

pub fn scroll_context_page_down(state: &mut AppState) {
    state.chat_context_scroll_from_bottom = state
        .chat_context_scroll_from_bottom
        .saturating_sub(CONTEXT_SCROLL_PAGE);
}

pub fn format_tokens(n: u32) -> String {
    if n >= 1_000_000 {
        format!("{:.2}M", n as f64 / 1_000_000.0)
    } else if n >= 10_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else if n >= 1_000 {
        format!("{:.2}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

pub fn context_usage_pct(used: u32, limit: u32) -> f64 {
    if limit == 0 {
        0.0
    } else {
        (used as f64 / limit as f64 * 100.0).clamp(0.0, 999.9)
    }
}

pub fn format_context_usage(used: u32, context_limit: u32) -> String {
    format!(
        "{} / {} ({:.1}%)",
        format_tokens(used),
        format_tokens(context_limit),
        context_usage_pct(used, context_limit)
    )
}

pub fn context_status_note(state: &AppState) -> String {
    if state.tab != crate::app::Tab::Chat {
        return String::new();
    }
    if let Some(snap) = &state.chat_context {
        return format!(
            " │ ctx {}",
            format_context_usage(snap.tokens_used, snap.context_limit)
        );
    }
    let limit = state.config.llm.context_limit;
    format!(" │ ctx — / {}", format_tokens(limit))
}

fn token_bar(used: u32, limit: u32, width: usize) -> String {
    let width = width.max(8);
    let pct = if limit == 0 {
        0.0
    } else {
        (used as f64 / limit as f64).clamp(0.0, 1.0)
    };
    let filled = (pct * width as f64).round() as usize;
    format!(
        "[{}{}]",
        "█".repeat(filled.min(width)),
        "░".repeat(width.saturating_sub(filled))
    )
}

fn role_style(th: ThemePalette, display_role: &str) -> Style {
    let fg = match display_role {
        "system" => th.muted,
        "assistant" => th.assistant,
        "tool" => th.accent_dim,
        "harness" => th.warn,
        "reasoning" => th.accent,
        "user" => th.accent,
        _ => th.text,
    };
    Style::default().fg(fg).add_modifier(Modifier::BOLD)
}

fn role_content_style(th: ThemePalette, display_role: &str) -> Style {
    let fg = match display_role {
        "system" => th.muted,
        "assistant" => th.assistant,
        "tool" => th.text,
        "harness" => th.muted,
        "reasoning" => th.accent_dim,
        "user" => th.text,
        _ => th.text,
    };
    Style::default().fg(fg)
}

pub fn format_message_tokens(n: u32) -> String {
    if n >= 10_000 {
        format!("{:.1}k tokens", n as f64 / 1_000.0)
    } else if n >= 1_000 {
        format!("{:.2}k tokens", n as f64 / 1_000.0)
    } else {
        format!("{n} tokens")
    }
}

fn render_message_content(
    th: ThemePalette,
    display_role: &str,
    content: &str,
    max_width: Option<usize>,
) -> Vec<Line<'static>> {
    let base = role_content_style(th, display_role);
    markdown_to_lines_in_width(th, content, base, max_width)
        .into_iter()
        .map(|line| {
            if line.spans.is_empty() {
                Line::from("")
            } else {
                let mut spans = vec![Span::raw("  ")];
                spans.extend(line.spans);
                Line::from(spans)
            }
        })
        .collect()
}

fn build_message_lines(
    th: ThemePalette,
    snapshot: Option<&ContextSnapshot>,
    max_width: Option<usize>,
) -> Vec<Line<'static>> {
    let Some(snap) = snapshot else {
        return vec![
            Line::from(""),
            Line::from(Span::styled(
                "  No context yet",
                Style::default().fg(th.muted).add_modifier(Modifier::ITALIC),
            )),
            Line::from(Span::styled(
                "  Send a message to populate the LLM context.",
                Style::default().fg(th.muted),
            )),
        ];
    };

    let mut lines = Vec::new();
    for (i, msg) in snap.messages.iter().enumerate() {
        if i > 0 {
            lines.push(Line::from(""));
        }
        lines.push(Line::from(vec![
            Span::styled(
                format!("[{}]", msg.display_role),
                role_style(th, &msg.display_role),
            ),
            Span::styled(
                format!(" {}", format_message_tokens(msg.tokens)),
                Style::default().fg(th.muted).add_modifier(Modifier::ITALIC),
            ),
        ]));
        lines.extend(render_message_content(
            th,
            &msg.display_role,
            &msg.content,
            max_width.map(|w| w.saturating_sub(2)),
        ));
    }
    lines
}

fn draw_context_header(
    frame: &mut ratatui::Frame,
    area: Rect,
    th: ThemePalette,
    snapshot: Option<&ContextSnapshot>,
    busy: bool,
    focused: bool,
) {
    let title = if busy {
        "LLM Context · live"
    } else {
        "LLM Context"
    };
    let title = if focused {
        format!("{title} ◀")
    } else {
        title.to_string()
    };
    let block = theme::pane_block(th, title, focused);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let Some(snap) = snapshot else {
        let line = Line::from(vec![Span::styled(
            "  — / —",
            Style::default().fg(th.muted),
        )]);
        frame.render_widget(Paragraph::new(line), inner);
        return;
    };

    let over = snap.tokens_used > snap.context_limit;
    let near_input_cap = snap.tokens_used > snap.input_budget;
    let usage_style = Style::default().fg(if over {
        th.err
    } else if near_input_cap {
        th.warn
    } else {
        th.text
    });

    let bar_w = inner.width.saturating_sub(2) as usize;
    let mut header = vec![
        Line::from(vec![
            Span::styled("context ", Style::default().fg(th.muted)),
            Span::styled(
                format_context_usage(snap.tokens_used, snap.context_limit),
                usage_style.add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![Span::styled(
            format!("  {}", token_bar(snap.tokens_used, snap.context_limit, bar_w)),
            Style::default().fg(if over { th.err } else { th.accent }),
        )]),
        Line::from(vec![
            Span::styled("input cap ", Style::default().fg(th.muted)),
            Span::styled(format_tokens(snap.input_budget), Style::default().fg(th.muted)),
            Span::styled("  ·  ", Style::default().fg(th.muted)),
            Span::styled(format!("{} msgs", snap.message_count), Style::default().fg(th.muted)),
            Span::styled("  ·  ", Style::default().fg(th.muted)),
            Span::styled(format!("step {}", snap.turn), Style::default().fg(th.muted)),
        ]),
    ];
    if header.len() as u16 > inner.height {
        header.truncate(inner.height as usize);
    }
    frame.render_widget(Paragraph::new(Text::from(header)), inner);
}

pub fn draw_context_panel(frame: &mut ratatui::Frame, area: Rect, state: &AppState) {
    let th = theme::ThemePalette::from_mode(state.config.tui.theme);
    let header_h = 4u16.min(area.height);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(header_h), Constraint::Min(0)])
        .split(area);

    draw_context_header(
        frame,
        chunks[0],
        th,
        state.chat_context.as_ref(),
        state.chat_busy,
        state.chat_pane_focus_is_context(),
    );

    let body_area = chunks[1];
    if body_area.height == 0 {
        return;
    }

    let block = Block::default()
        .borders(Borders::LEFT | Borders::RIGHT | Borders::BOTTOM)
        .border_style(Style::default().fg(th.border))
        .style(Style::default().bg(th.panel));
    let inner = block.inner(body_area);
    frame.render_widget(block, body_area);

    let lines = build_message_lines(
        th,
        state.chat_context.as_ref(),
        Some(inner.width.max(1) as usize),
    );
    let width = inner.width.max(1);
    let paragraph = Paragraph::new(Text::from(lines))
        .style(Style::default().bg(th.panel))
        .wrap(Wrap { trim: false });

    let visible = inner.height.max(1);
    let total = paragraph.line_count(width) as u16;
    let max_scroll = total.saturating_sub(visible);
    let scroll_from_bottom = state
        .chat_context_scroll_from_bottom
        .min(max_scroll);
    let scroll_y = max_scroll.saturating_sub(scroll_from_bottom);

    frame.render_widget(paragraph.scroll((scroll_y, 0)), inner);

    if total > visible {
        let mut sb_state = paragraph_scrollbar_state(total, visible, scroll_y);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .thumb_symbol("█")
                .track_symbol(Some("░"))
                .thumb_style(Style::default().fg(th.accent))
                .track_style(Style::default().fg(th.muted)),
            inner,
            &mut sb_state,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::chat_loop::{ContextLine, ContextSnapshot};
    use ratatui::style::Modifier;

    #[test]
    fn format_context_usage_shows_limit_and_pct() {
        assert_eq!(format_context_usage(12_400, 64_000), "12.4k / 64.0k (19.4%)");
    }

    #[test]
    fn token_bar_fills_proportionally() {
        let bar = token_bar(32_000, 64_000, 10);
        assert!(bar.starts_with("[█████"));
    }

    #[test]
    fn context_message_body_renders_markdown() {
        let th = theme::ThemePalette::dark();
        let snap = ContextSnapshot {
            turn: 2,
            tokens_used: 100,
            input_budget: 40_000,
            context_limit: 64_000,
            message_count: 1,
            messages: vec![ContextLine {
                display_role: "assistant".into(),
                content: "**PR #19264** — CI failing\n- check logs\n- retry".into(),
                tokens: 42,
            }],
        };
        let lines = build_message_lines(th, Some(&snap), Some(72));
        let joined = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.clone())
            .collect::<String>();
        assert!(joined.contains("PR #19264"));
        assert!(
            lines.iter().any(|l| {
                l.spans
                    .iter()
                    .any(|s| s.style.add_modifier.contains(Modifier::BOLD))
            }),
            "expected bold markdown span in context body"
        );
    }

    #[test]
    fn format_message_tokens_readable() {
        assert_eq!(format_message_tokens(842), "842 tokens");
        assert_eq!(format_message_tokens(1_240), "1.24k tokens");
    }

    #[test]
    fn context_panel_wraps_long_plain_text() {
        use ratatui::widgets::{Paragraph, Wrap};
        let th = theme::ThemePalette::dark();
        let snap = ContextSnapshot {
            turn: 1,
            tokens_used: 50,
            input_budget: 40_000,
            context_limit: 64_000,
            message_count: 1,
            messages: vec![ContextLine {
                display_role: "system".into(),
                content: "word ".repeat(60),
                tokens: 10,
            }],
        };
        let width = 48u16;
        let lines = build_message_lines(th, Some(&snap), Some(width as usize));
        let p = Paragraph::new(Text::from(lines))
            .wrap(Wrap { trim: false });
        assert!(
            p.line_count(width) > 1,
            "non-table context body should wrap in the panel"
        );
    }

    #[test]
    fn context_panel_wraps_long_markdown() {
        use ratatui::widgets::{Paragraph, Wrap};
        let th = theme::ThemePalette::dark();
        let snap = ContextSnapshot {
            turn: 1,
            tokens_used: 50,
            input_budget: 40_000,
            context_limit: 64_000,
            message_count: 1,
            messages: vec![ContextLine {
                display_role: "assistant".into(),
                content: format!("**Summary:** {}", "detail ".repeat(40)),
                tokens: 10,
            }],
        };
        let width = 48u16;
        let lines = build_message_lines(th, Some(&snap), Some(width as usize));
        let p = Paragraph::new(Text::from(lines))
            .wrap(Wrap { trim: false });
        assert!(
            p.line_count(width) > 1,
            "markdown paragraphs should wrap in the panel"
        );
    }

    #[test]
    fn context_panel_table_rows_stay_single_line() {
        use ratatui::widgets::{Paragraph, Wrap};
        let th = theme::ThemePalette::dark();
        let snap = ContextSnapshot {
            turn: 1,
            tokens_used: 50,
            input_budget: 40_000,
            context_limit: 64_000,
            message_count: 1,
            messages: vec![ContextLine {
                display_role: "tool".into(),
                content: "| Tool | Notes |\n|------|-------|\n| pr_get_overview | snapshot |\n| pr_list_open | list |".into(),
                tokens: 10,
            }],
        };
        let width = 36u16;
        let lines = build_message_lines(th, Some(&snap), Some(width as usize));
        let table_line_count = lines
            .iter()
            .filter(|l| {
                let text: String = l.spans.iter().map(|s| s.content.as_ref()).collect();
                let t = text.trim();
                t.starts_with('│') || (t.starts_with('├') && t.ends_with('┤'))
            })
            .count();
        assert!(table_line_count >= 2, "expected formatted table rows");
        let p = Paragraph::new(Text::from(lines))
            .wrap(Wrap { trim: false });
        assert!(
            p.line_count(width) >= table_line_count,
            "table rows should not be word-wrapped into extra lines"
        );
    }
}
