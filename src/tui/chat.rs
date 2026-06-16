use std::sync::Mutex;

use ratatui::layout::{Constraint, Direction, Layout, Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Paragraph, Scrollbar, ScrollbarOrientation, Wrap};
use unicode_width::UnicodeWidthStr;

use crate::app::AppState;
use crate::tui::context_panel::draw_context_panel;
use crate::tui::scroll::paragraph_scrollbar_state;
use crate::tui::spinner;
use crate::tui::theme::{self, ThemePalette};

const CHAT_SCROLL_PAGE: u16 = 8;
const INPUT_PREFIX: &str = "▸ ";

struct CachedMessageEntry {
    source: String,
    lines: Vec<Line<'static>>,
}

struct ChatRenderCache {
    revision: u64,
    width: u16,
    entries: Vec<CachedMessageEntry>,
}

static RENDER_CACHE: Mutex<ChatRenderCache> = Mutex::new(ChatRenderCache {
    revision: 0,
    width: 0,
    entries: Vec::new(),
});

fn palette(state: &AppState) -> ThemePalette {
    ThemePalette::from_mode(state.config.tui.theme)
}

pub fn scroll_page_up(state: &mut AppState) {
    state.chat_scroll_from_bottom = state.chat_scroll_from_bottom.saturating_add(CHAT_SCROLL_PAGE);
}

pub fn scroll_page_down(state: &mut AppState) {
    state.chat_scroll_from_bottom = state.chat_scroll_from_bottom.saturating_sub(CHAT_SCROLL_PAGE);
}

fn welcome_lines(th: ThemePalette) -> Vec<Line<'static>> {
    vec![
        Line::from(""),
        Line::from(Span::styled(
            "  Welcome to unistar-coworker",
            Style::default()
                .fg(th.accent)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            "  Ask about PRs, CI, reviews, or digests.",
            Style::default().fg(th.muted).add_modifier(Modifier::ITALIC),
        )),
        Line::from(Span::styled(
            "  e.g.  list open PRs with failing CI",
            Style::default().fg(th.muted),
        )),
    ]
}

fn tool_pending_lines(th: ThemePalette, label: &str) -> Vec<Line<'static>> {
    vec![Line::from(vec![
        Span::raw("      "),
        Span::styled("◔ ", Style::default().fg(th.tool)),
        Span::styled(
            label.to_string(),
            Style::default().fg(th.tool).add_modifier(Modifier::ITALIC),
        ),
        Span::styled(" …", Style::default().fg(th.muted)),
    ])]
}

fn activity_status_line(th: ThemePalette, label: &str) -> Line<'static> {
    Line::from(vec![
        Span::raw("      "),
        Span::styled(
            format!("{} ", spinner::frame_char()),
            Style::default().fg(th.accent),
        ),
        Span::styled(
            label.to_string(),
            Style::default()
                .fg(th.accent)
                .add_modifier(Modifier::ITALIC),
        ),
        Span::styled(" …", Style::default().fg(th.muted)),
    ])
}

fn thinking_status_line(th: ThemePalette) -> Line<'static> {
    activity_status_line(th, "waiting for model")
}

fn tool_running_lines(th: ThemePalette, name: &str) -> Vec<Line<'static>> {
    vec![activity_status_line(th, &format!("running {name}"))]
}

fn reasoning_compressing_line(th: ThemePalette) -> Line<'static> {
    activity_status_line(th, "summarizing reasoning")
}

fn reasoning_stream_tail(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let count = trimmed.chars().count();
    if count <= max_chars {
        return trimmed.to_string();
    }
    let skip = count.saturating_sub(max_chars.saturating_sub(1));
    format!(
        "…{}",
        trimmed.chars().skip(skip).collect::<String>()
    )
}

fn reasoning_body_lines(th: ThemePalette, text: &str) -> Vec<Line<'static>> {
    const MAX_LINES: usize = 8;
    const MAX_CHARS_PER_LINE: usize = 96;

    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    let body_style = Style::default()
        .fg(th.accent_dim)
        .add_modifier(Modifier::ITALIC);
    let indent = "        ";

    let segments: Vec<String> = if trimmed.contains('\n') {
        trimmed
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(|line| crate::agent::context::truncate_chars(line, MAX_CHARS_PER_LINE))
            .collect()
    } else {
        let tail = reasoning_stream_tail(trimmed, MAX_CHARS_PER_LINE * MAX_LINES);
        vec![crate::agent::context::truncate_chars(&tail, MAX_CHARS_PER_LINE)]
    };

    let start = segments.len().saturating_sub(MAX_LINES);
    segments[start..]
        .iter()
        .map(|line| {
            Line::from(vec![
                Span::raw(indent),
                Span::styled(line.clone(), body_style),
            ])
        })
        .collect()
}

fn reasoning_preview_lines(th: ThemePalette, text: &str) -> Vec<Line<'static>> {
    let mut out = vec![activity_status_line(th, "reasoning")];
    out.extend(reasoning_body_lines(th, text));
    out
}

fn streaming_preview_lines(th: ThemePalette, text: &str, max_width: usize) -> Vec<Line<'static>> {
    let mut out = vec![activity_status_line(th, "streaming reply")];
    out.extend(theme::format_assistant_tail_body_lines(
        th,
        text,
        Some(max_width),
    ));
    out
}

fn chat_shows_thinking_spinner(state: &AppState) -> bool {
    state.chat_busy
        && state.chat_streaming.is_none()
        && state.chat_tool_pending.is_none()
        && state.chat_tool_running.is_none()
        && state.chat_reasoning.is_none()
        && !state.chat_reasoning_compressing
}

fn sync_message_entries(
    th: ThemePalette,
    state: &AppState,
    entries: &mut Vec<CachedMessageEntry>,
    max_width: usize,
) {
    if entries.len() > state.chat_lines.len() {
        entries.truncate(state.chat_lines.len());
    }
    for (i, source) in state.chat_lines.iter().enumerate() {
        let stale = entries.get(i).is_none_or(|entry| entry.source != *source);
        if !stale {
            continue;
        }
        let lines = theme::format_chat_lines(th, source, Some(max_width));
        if i < entries.len() {
            entries[i] = CachedMessageEntry {
                source: source.clone(),
                lines,
            };
        } else {
            entries.push(CachedMessageEntry {
                source: source.clone(),
                lines,
            });
        }
    }
}

fn should_skip_tool_transcript_echo(entries: &[CachedMessageEntry], index: usize) -> bool {
    let Some(rest) = entries
        .get(index)
        .and_then(|e| e.source.strip_prefix("assistant> "))
    else {
        return false;
    };
    if !crate::agent::context::is_tool_result_transcript(rest) {
        return false;
    }
    let Some((tool_name, _)) = crate::agent::context::split_tool_transcript(rest) else {
        return false;
    };
    entries[..index].iter().rev().any(|entry| {
        let src = entry.source.as_str();
        src.starts_with("  ✓ ")
            && (src.contains(&format!(" {tool_name}("))
                || src.contains(&format!(" {tool_name} ("))
                || src.ends_with(&format!(" {tool_name}")))
    })
}

fn entry_compose_lines(
    th: ThemePalette,
    entry: &CachedMessageEntry,
    index: usize,
    state: &AppState,
) -> Vec<Line<'static>> {
    let mut lines = entry.lines.clone();
    if state.chat_expanded_tool_lines.contains(&index) {
        if let Some(body) = state.chat_tool_outputs.get(&index) {
            lines.extend(theme::format_tool_detail_lines(th, body));
        }
    }
    lines
}

fn wrapped_line_count(lines: &[Line], width: u16) -> u16 {
    if lines.is_empty() {
        return 0;
    }
    Paragraph::new(Text::from(lines.to_vec()))
        .wrap(Wrap { trim: false })
        .line_count(width.max(1)) as u16
}

fn tail_status_lines(th: ThemePalette, state: &AppState, panel_width: u16) -> Vec<Line<'static>> {
    if !(state.chat_busy
        || state.chat_streaming.is_some()
        || state.chat_tool_pending.is_some()
        || state.chat_tool_running.is_some()
        || state.chat_reasoning.is_some()
        || state.chat_reasoning_compressing)
    {
        return Vec::new();
    }
    let mut lines = Vec::new();
    if state.chat_reasoning_compressing {
        lines.push(reasoning_compressing_line(th));
    } else if let Some(ref reasoning) = state.chat_reasoning {
        lines.extend(reasoning_preview_lines(th, reasoning));
    }
    if let Some(ref pending) = state.chat_tool_pending {
        if !lines.is_empty() {
            lines.push(Line::from(""));
        }
        lines.extend(tool_pending_lines(th, pending));
    } else if let Some(ref name) = state.chat_tool_running {
        if !lines.is_empty() {
            lines.push(Line::from(""));
        }
        lines.extend(tool_running_lines(th, name));
    } else if let Some(ref partial) = state.chat_streaming {
        if !lines.is_empty() {
            lines.push(Line::from(""));
        }
        lines.extend(streaming_preview_lines(
            th,
            partial,
            theme::tail_content_max_width(panel_width),
        ));
    } else if chat_shows_thinking_spinner(state) {
        if !lines.is_empty() {
            lines.push(Line::from(""));
        }
        lines.push(thinking_status_line(th));
    }
    lines
}

fn compose_chat_lines(
    th: ThemePalette,
    state: &AppState,
    entries: &[CachedMessageEntry],
    panel_width: u16,
) -> Vec<Line<'static>> {
    if entries.is_empty()
        && state.chat_streaming.is_none()
        && state.chat_tool_pending.is_none()
        && state.chat_reasoning.is_none()
    {
        return welcome_lines(th);
    }

    let mut lines: Vec<Line> = Vec::new();
    for (i, entry) in entries.iter().enumerate() {
        if should_skip_tool_transcript_echo(entries, i) {
            continue;
        }
        let composed = entry_compose_lines(th, entry, i, state);
        if composed.is_empty() {
            continue;
        }
        if !lines.is_empty() {
            lines.push(Line::from(""));
        }
        lines.extend(composed);
    }

    if state.chat_busy
        || state.chat_streaming.is_some()
        || state.chat_tool_pending.is_some()
        || state.chat_reasoning.is_some()
        || state.chat_reasoning_compressing
    {
        if !lines.is_empty() {
            lines.push(Line::from(""));
        }
        lines.extend(tail_status_lines(th, state, panel_width));
    }
    lines
}

struct ChatViewport {
    lines: Vec<Line<'static>>,
    total_height: u16,
}

fn chat_viewport(th: ThemePalette, state: &AppState, width: u16) -> ChatViewport {
    let mut cache = RENDER_CACHE.lock().expect("chat render cache");
    let stale = cache.revision != state.chat_render_revision || cache.width != width;
    if stale {
        sync_message_entries(
            th,
            state,
            &mut cache.entries,
            theme::message_content_max_width(width),
        );
        cache.revision = state.chat_render_revision;
        cache.width = width;
    }
    let lines = compose_chat_lines(th, state, &cache.entries, width);
    let total = wrapped_line_count(&lines, width);
    ChatViewport {
        lines,
        total_height: total,
    }
}

pub fn chat_pane_rects(area: Rect, context_visible: bool) -> (Rect, Option<Rect>) {
    if context_visible {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(62), Constraint::Percentage(38)])
            .split(area);
        (chunks[0], Some(chunks[1]))
    } else {
        (area, None)
    }
}

/// Map a click to Messages vs Context when the split is visible.
pub fn focus_pane_at(
    content_area: Rect,
    context_visible: bool,
    column: u16,
    row: u16,
) -> Option<crate::app::ChatPaneFocus> {
    if !context_visible {
        return None;
    }
    let (messages, Some(context)) = chat_pane_rects(content_area, true) else {
        return None;
    };
    if context.contains(ratatui::layout::Position::new(column, row)) {
        return Some(crate::app::ChatPaneFocus::Context);
    }
    if messages.contains(ratatui::layout::Position::new(column, row)) {
        return Some(crate::app::ChatPaneFocus::Messages);
    }
    None
}

pub fn draw_chat(frame: &mut ratatui::Frame, area: Rect, state: &AppState) {
    let (messages_area, context_area) = chat_pane_rects(area, state.chat_context_visible);
    if let Some(context_area) = context_area {
        draw_chat_pane(frame, messages_area, state);
        draw_context_panel(frame, context_area, state);
    } else {
        draw_chat_pane(frame, messages_area, state);
    }
}

fn draw_chat_pane(frame: &mut ratatui::Frame, area: Rect, state: &AppState) {
    let th = palette(state);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(3)])
        .split(area);

    let busy_suffix = if state.chat_busy {
        if state.chat_streaming.is_some() {
            " · streaming"
        } else if state.chat_tool_running.is_some() || state.chat_tool_pending.is_some() {
            " · tool"
        } else if state.chat_reasoning_compressing {
            " · summarizing"
        } else if state.chat_reasoning.is_some() {
            " · reasoning"
        } else {
            " · model"
        }
    } else {
        ""
    };
    let messages_focused = !state.chat_context_visible
        || state.chat_pane_focus == crate::app::ChatPaneFocus::Messages;
    let focus_mark = if state.chat_context_visible && messages_focused {
        " ◀"
    } else {
        ""
    };
    let history_block = theme::pane_block(th, format!("Messages{busy_suffix}{focus_mark}"), messages_focused);
    let inner = history_block.inner(chunks[0]);

    let visible = inner.height.max(1);
    let vp = chat_viewport(th, state, inner.width);
    let total = vp.total_height;
    let max_scroll = total.saturating_sub(visible);
    let scroll_from_bottom = state.chat_scroll_from_bottom.min(max_scroll);
    let scroll_y = max_scroll.saturating_sub(scroll_from_bottom);

    frame.render_widget(history_block, chunks[0]);
    frame.render_widget(
        Paragraph::new(Text::from(vp.lines))
            .style(Style::default().bg(th.panel))
            .wrap(Wrap { trim: false })
            .scroll((scroll_y, 0)),
        inner,
    );

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

    let focused = !state.chat_busy;
    let input_block = theme::input_block(th, "Input", focused);
    let input_area = chunks[1];
    let input_inner = input_block.inner(input_area);

    let input_line = if state.chat_busy {
        let label = if state.chat_streaming.is_some() {
            "streaming reply…"
        } else if state.chat_tool_running.is_some() {
            "running tool…"
        } else if state.chat_tool_pending.is_some() {
            "preparing tool…"
        } else if state.chat_reasoning_compressing {
            "summarizing reasoning…"
        } else if state.chat_reasoning.is_some() {
            "reasoning…"
        } else if chat_shows_thinking_spinner(state) {
            "waiting for model…"
        } else {
            "busy…"
        };
        Line::from(vec![
            Span::raw("  "),
            Span::styled(
                format!("{} ", spinner::frame_char()),
                Style::default().fg(th.muted),
            ),
            Span::styled(label, Style::default().fg(th.muted).add_modifier(Modifier::ITALIC)),
        ])
    } else {
        Line::from(vec![
            Span::raw("  "),
            Span::styled(INPUT_PREFIX, Style::default().fg(th.accent)),
            Span::styled(state.chat_input.clone(), Style::default().fg(th.text)),
        ])
    };
    frame.render_widget(
        Paragraph::new(input_line).block(input_block),
        input_area,
    );

    if focused {
        let cursor_x = input_inner.x.saturating_add(
            (2 + INPUT_PREFIX.width() + state.chat_input.width()) as u16,
        );
        frame.set_cursor_position(Position::new(cursor_x, input_inner.y));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrapped_line_count_matches_paragraph() {
        let lines = vec![
            Line::from("Hello world this is a long line that should wrap across cells"),
            Line::from("short"),
        ];
        let width = 12;
        let p = Paragraph::new(Text::from(lines.clone())).wrap(Wrap { trim: false });
        assert_eq!(p.line_count(width), 7);
    }

    #[test]
    fn reasoning_stream_tail_keeps_latest_chars() {
        let text = "alpha beta gamma delta epsilon";
        let tail = reasoning_stream_tail(text, 12);
        assert!(tail.starts_with('…'));
        assert!(tail.contains("epsilon"));
    }

    #[test]
    fn streaming_preview_matches_activity_header_style() {
        let th = ThemePalette::dark();
        let rows = streaming_preview_lines(th, "**Hello** world", 72);
        assert!(!rows.is_empty());
        let header: String = rows[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(header.contains("streaming reply"));
        assert!(header.contains('…'));
        assert!(rows.len() >= 2);
        let body: String = rows[1].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(body.contains("Hello"));
    }

    #[test]
    fn reasoning_preview_matches_thinking_header_style() {
        let th = ThemePalette::dark();
        let rows = reasoning_preview_lines(th, "Checking CI on PR #42");
        assert!(!rows.is_empty());
        let header: String = rows[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(header.contains("reasoning"));
        assert!(header.contains('…'));
        assert!(rows.len() >= 2);
    }

    #[test]
    fn scrollbar_max_scroll_at_bottom_pin() {
        let lines = vec![
            Line::from("Hello world this is a long line that should wrap across cells"),
            Line::from("short"),
        ];
        let width = 12u16;
        let visible = 3u16;
        let total = wrapped_line_count(&lines, width);
        let max_scroll = total.saturating_sub(visible);
        let scroll_y = max_scroll;
        assert_eq!(scroll_y as usize + visible as usize, total as usize);
    }

    #[test]
    fn skips_duplicate_tool_transcript_after_tool_row() {
        let entries = vec![
            CachedMessageEntry {
                source: "  ✓ pr_list_changed_files(repo=acme/widget, pr_number=19275) (120ms)".into(),
                lines: vec![],
            },
            CachedMessageEntry {
                source: "assistant> tool_result(pr_list_changed_files, pr_number=19275):\n1 changed file(s)"
                    .into(),
                lines: vec![],
            },
        ];
        assert!(should_skip_tool_transcript_echo(&entries, 1));
        assert!(!should_skip_tool_transcript_echo(&entries, 0));
    }

    #[test]
    fn assistant_message_table_rows_stay_single_line() {
        use ratatui::widgets::{Paragraph, Wrap};
        let th = ThemePalette::dark();
        let width = 48u16;
        let max_width = theme::message_content_max_width(width);
        let body = "| PR | CI | Review |\n|----|----|--------|\n| #19274 | failing | pending |\n| #19273 | ok | approved |";
        let rows = theme::format_assistant_message_lines(th, body, Some(max_width));
        let table_line_count = rows
            .iter()
            .filter(|l| {
                let text: String = l.spans.iter().map(|s| s.content.as_ref()).collect();
                let t = text.trim();
                t.starts_with('│') || (t.starts_with('├') && t.ends_with('┤'))
            })
            .count();
        assert!(table_line_count >= 2, "expected formatted table rows");
        let p = Paragraph::new(Text::from(rows))
            .wrap(Wrap { trim: false });
        assert!(
            p.line_count(width) >= table_line_count,
            "table rows should not be word-wrapped into extra lines"
        );
    }

    #[test]
    fn focus_pane_at_respects_split() {
        use crate::app::ChatPaneFocus;
        let area = Rect::new(0, 5, 100, 20);
        let (messages, context) = chat_pane_rects(area, true);
        let context = context.expect("context pane");
        assert_eq!(
            focus_pane_at(area, true, messages.x + 2, messages.y + 2),
            Some(ChatPaneFocus::Messages)
        );
        assert_eq!(
            focus_pane_at(area, true, context.x + 2, context.y + 2),
            Some(ChatPaneFocus::Context)
        );
        assert_eq!(focus_pane_at(area, false, messages.x + 2, messages.y + 2), None);
    }
}
