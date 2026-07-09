use std::sync::{LazyLock, Mutex};

use ratatui::layout::{Constraint, Direction, Layout, Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Clear, Paragraph, Scrollbar, ScrollbarOrientation};
use unicode_width::UnicodeWidthStr;

use crate::context_panel::draw_context_panel;
use crate::markdown::StreamingMarkdownRenderer;
use crate::scroll::paragraph_scrollbar_state;
use crate::spinner;
use crate::theme::{self, ThemePalette};
use coworker_core::agent::chat_loop::ActivityFlowKind;
use coworker_core::agent::chat_loop::ChatActivityFlow;
use coworker_core::app::AppState;

const CHAT_SCROLL_PAGE: u16 = 8;
const INPUT_PREFIX: &str = "▸ ";

struct CachedMessageEntry {
    source: String,
    format_width: u16,
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

static STREAMING_MD: LazyLock<Mutex<StreamingMarkdownRenderer>> =
    LazyLock::new(|| Mutex::new(StreamingMarkdownRenderer::new()));

pub fn reset_streaming_markdown_cache() {
    if let Ok(mut cache) = STREAMING_MD.lock() {
        cache.clear();
    }
}

fn palette(state: &AppState) -> ThemePalette {
    ThemePalette::from_tui(&state.config.tui, state.config.theme())
}

pub fn scroll_page_up(state: &mut AppState) {
    state.chat_scroll_from_bottom = state
        .chat_scroll_from_bottom
        .saturating_add(CHAT_SCROLL_PAGE);
}

pub fn scroll_page_down(state: &mut AppState) {
    state.chat_scroll_from_bottom = state
        .chat_scroll_from_bottom
        .saturating_sub(CHAT_SCROLL_PAGE);
}

fn welcome_lines(th: ThemePalette) -> Vec<Line<'static>> {
    vec![
        Line::from(""),
        Line::from(Span::styled(
            "  Welcome to unistar-coworker",
            Style::default().fg(th.accent).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            "  Ask about your workspace — code, tests, or docs.",
            Style::default().fg(th.muted).add_modifier(Modifier::ITALIC),
        )),
        Line::from(Span::styled(
            "  e.g.  explain src/main.rs · run the unit tests",
            Style::default().fg(th.muted),
        )),
        Line::from(Span::styled(
            "  GitHub: name owner/repo or paste a PR URL (no default repo).",
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

fn tool_running_lines(th: ThemePalette, name: &str, detail: Option<&str>) -> Vec<Line<'static>> {
    let label = match detail {
        Some(d) if !d.is_empty() => format!("running {name} ({d})"),
        _ => format!("running {name}"),
    };
    vec![activity_status_line(th, &label)]
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
    format!("…{}", trimmed.chars().skip(skip).collect::<String>())
}

fn reasoning_tail_source(text: &str, panel_width: u16) -> String {
    const MAX_SOURCE_LINES: usize = 8;

    let trimmed = text.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let max_width = theme::tail_content_max_width(panel_width);
    if trimmed.contains('\n') {
        let lines: Vec<&str> = trimmed
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect();
        let start = lines.len().saturating_sub(MAX_SOURCE_LINES);
        lines[start..].join("\n")
    } else {
        reasoning_stream_tail(
            trimmed,
            max_width.saturating_mul(MAX_SOURCE_LINES).saturating_mul(4),
        )
    }
}

const REASONING_TAIL_MAX_ROWS: usize = 8;

fn tail_body_lines(
    text: String,
    style: Style,
    indent: &str,
    _panel_width: u16,
    max_rows: usize,
) -> Vec<Line<'static>> {
    if text.is_empty() {
        return Vec::new();
    }

    let mut logical: Vec<Line<'static>> = if text.contains('\n') {
        text.lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(|line| {
                Line::from(vec![
                    Span::raw(indent.to_string()),
                    Span::styled(line.to_string(), style),
                ])
            })
            .collect()
    } else {
        vec![Line::from(vec![
            Span::raw(indent.to_string()),
            Span::styled(text, style),
        ])]
    };

    if logical.len() > max_rows {
        logical.split_off(logical.len() - max_rows)
    } else {
        logical
    }
}

fn reasoning_body_lines(th: ThemePalette, text: &str, panel_width: u16) -> Vec<Line<'static>> {
    let source = reasoning_tail_source(text, panel_width);
    let body_style = Style::default()
        .fg(th.accent_dim)
        .add_modifier(Modifier::ITALIC);
    tail_body_lines(
        source,
        body_style,
        "        ",
        panel_width,
        REASONING_TAIL_MAX_ROWS,
    )
}

fn activity_flow_label(kind: ActivityFlowKind) -> &'static str {
    match kind {
        ActivityFlowKind::Skill => "skill",
        ActivityFlowKind::Github => "github",
    }
}

fn activity_flow_preview_lines(
    th: ThemePalette,
    flow: &ChatActivityFlow,
    panel_width: u16,
) -> Vec<Line<'static>> {
    let mut out = vec![activity_status_line(th, activity_flow_label(flow.kind))];
    out.extend(reasoning_body_lines(th, &flow.text, panel_width));
    out
}

fn reasoning_preview_lines(th: ThemePalette, text: &str, panel_width: u16) -> Vec<Line<'static>> {
    let mut out = vec![activity_status_line(th, "reasoning")];
    out.extend(reasoning_body_lines(th, text, panel_width));
    out
}

fn indent_body_lines(indent: &str, lines: Vec<Line<'static>>) -> Vec<Line<'static>> {
    lines
        .into_iter()
        .map(|line| {
            if line.spans.is_empty() {
                Line::from("")
            } else {
                let mut spans = vec![Span::raw(indent.to_string())];
                spans.extend(line.spans);
                Line::from(spans)
            }
        })
        .collect()
}

fn streaming_preview_lines(th: ThemePalette, text: &str, panel_width: u16) -> Vec<Line<'static>> {
    let body = theme::normalize_message_layout(text);
    let max_width = theme::tail_content_max_width(panel_width);
    let content_style = Style::default().fg(th.assistant);
    let mut cache = STREAMING_MD.lock().expect("streaming markdown cache");
    let md_lines = cache.render(th, &body, content_style, Some(max_width));
    let mut out = vec![activity_status_line(th, "streaming reply")];
    out.extend(indent_body_lines("        ", md_lines));
    out
}

fn chat_shows_thinking_spinner(state: &AppState) -> bool {
    state.chat_busy
        && state.chat_streaming.is_none()
        && state.chat_tool_pending.is_none()
        && state.chat_tool_running.is_none()
        && state.chat_reasoning.is_none()
        && state.chat_activity_flow.is_none()
        && !state.chat_reasoning_compressing
}

fn should_skip_reasoning_transcript(entry: &CachedMessageEntry) -> bool {
    entry.source.starts_with("  … reasoning:")
}

fn sync_message_entries(
    th: ThemePalette,
    state: &AppState,
    entries: &mut Vec<CachedMessageEntry>,
    panel_width: u16,
) {
    if entries.len() > state.chat_lines.len() {
        entries.truncate(state.chat_lines.len());
    }
    let content_w = theme::chat_content_max_width(panel_width);
    for (i, source) in state.chat_lines.iter().enumerate() {
        let stale = entries
            .get(i)
            .is_none_or(|entry| entry.source != *source || entry.format_width != panel_width);
        if !stale {
            continue;
        }
        let lines = theme::format_chat_lines(th, source, Some(content_w));
        if i < entries.len() {
            entries[i] = CachedMessageEntry {
                source: source.clone(),
                format_width: panel_width,
                lines,
            };
        } else {
            entries.push(CachedMessageEntry {
                source: source.clone(),
                format_width: panel_width,
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
    if !coworker_core::agent::context::is_tool_result_transcript(rest) {
        return false;
    }
    let Some((tool_name, _)) = coworker_core::agent::context::split_tool_transcript(rest) else {
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

fn is_tool_activity_source(source: &str) -> bool {
    source.starts_with("  → ")
        || source.starts_with("  ✓ ")
        || source.starts_with("  ✗ ")
        || source.starts_with("  ⚠ ")
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ToolGroupPos {
    None,
    Single,
    First,
    Middle,
    Last,
}

fn entry_visible_in_history(entries: &[CachedMessageEntry], index: usize) -> bool {
    !should_skip_reasoning_transcript(&entries[index])
        && !should_skip_tool_transcript_echo(entries, index)
}

fn prev_visible_entry_index(entries: &[CachedMessageEntry], index: usize) -> Option<usize> {
    (0..index)
        .rev()
        .find(|&i| entry_visible_in_history(entries, i))
}

fn next_visible_entry_index(entries: &[CachedMessageEntry], index: usize) -> Option<usize> {
    ((index + 1)..entries.len()).find(|&i| entry_visible_in_history(entries, i))
}

fn tool_group_pos(entries: &[CachedMessageEntry], index: usize) -> ToolGroupPos {
    if !is_tool_activity_source(&entries[index].source) {
        return ToolGroupPos::None;
    }
    let has_prev = prev_visible_entry_index(entries, index)
        .is_some_and(|i| is_tool_activity_source(&entries[i].source));
    let has_next = next_visible_entry_index(entries, index)
        .is_some_and(|i| is_tool_activity_source(&entries[i].source));
    match (has_prev, has_next) {
        (false, false) => ToolGroupPos::Single,
        (false, true) => ToolGroupPos::First,
        (true, true) => ToolGroupPos::Middle,
        (true, false) => ToolGroupPos::Last,
    }
}

fn apply_tool_group_connector(lines: Vec<Line<'static>>, pos: ToolGroupPos) -> Vec<Line<'static>> {
    if !matches!(
        pos,
        ToolGroupPos::First | ToolGroupPos::Middle | ToolGroupPos::Last
    ) {
        return lines;
    }
    lines
        .into_iter()
        .map(|mut line| {
            if let Some(span) = line.spans.first_mut() {
                if span.content.as_ref() == "      " {
                    span.content = "      │ ".into();
                }
            }
            line
        })
        .collect()
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

fn panel_line_count(lines: &[Line], _width: u16) -> u16 {
    if lines.is_empty() {
        return 0;
    }
    lines.len().min(u16::MAX as usize) as u16
}

fn tail_status_lines(th: ThemePalette, state: &AppState, panel_width: u16) -> Vec<Line<'static>> {
    if !(state.chat_busy
        || state.chat_streaming.is_some()
        || state.chat_tool_pending.is_some()
        || state.chat_tool_running.is_some()
        || state.chat_reasoning.is_some()
        || state.chat_activity_flow.is_some()
        || state.chat_reasoning_compressing)
    {
        return Vec::new();
    }
    let mut lines = Vec::new();
    if state.chat_reasoning_compressing {
        lines.push(reasoning_compressing_line(th));
    }
    if let Some(ref reasoning) = state.chat_reasoning {
        if state.chat_reasoning_compressing {
            lines.extend(reasoning_body_lines(th, reasoning, panel_width));
        } else {
            lines.extend(reasoning_preview_lines(th, reasoning, panel_width));
        }
    }
    if let Some(ref flow) = state.chat_activity_flow {
        if !lines.is_empty() {
            lines.push(Line::from(""));
        }
        lines.extend(activity_flow_preview_lines(th, flow, panel_width));
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
        lines.extend(tool_running_lines(
            th,
            name,
            state.chat_tool_running_detail.as_deref(),
        ));
    } else if let Some(ref partial) = state.chat_streaming {
        if !lines.is_empty() {
            lines.push(Line::from(""));
        }
        lines.extend(streaming_preview_lines(th, partial, panel_width));
    } else if chat_shows_thinking_spinner(state) {
        if !lines.is_empty() {
            lines.push(Line::from(""));
        }
        lines.push(thinking_status_line(th));
    }
    lines
}

fn compose_history_lines(
    th: ThemePalette,
    state: &AppState,
    entries: &[CachedMessageEntry],
    panel_width: u16,
) -> Vec<Line<'static>> {
    if entries.is_empty()
        && state.chat_streaming.is_none()
        && state.chat_tool_pending.is_none()
        && !state.chat_busy
    {
        return welcome_lines(th);
    }

    let mut lines: Vec<Line> = Vec::new();
    for (i, entry) in entries.iter().enumerate() {
        if should_skip_reasoning_transcript(entry) {
            continue;
        }
        if should_skip_tool_transcript_echo(entries, i) {
            continue;
        }
        let group_pos = tool_group_pos(entries, i);
        let composed =
            apply_tool_group_connector(entry_compose_lines(th, entry, i, state), group_pos);
        if composed.is_empty() {
            continue;
        }
        let skip_gap = matches!(group_pos, ToolGroupPos::Middle | ToolGroupPos::Last);
        if !lines.is_empty() && !skip_gap {
            lines.push(Line::from(""));
        }
        lines.extend(composed);
    }
    let pad_style = Style::default().bg(th.panel);
    crate::markdown::finalize_panel_lines(lines, panel_width, pad_style, true)
}

const MAX_LIVE_STATUS_ROWS: usize = 12;

fn compose_live_status_lines(
    th: ThemePalette,
    state: &AppState,
    panel_width: u16,
) -> Vec<Line<'static>> {
    let raw = tail_status_lines(th, state, panel_width);
    if raw.is_empty() {
        return Vec::new();
    }
    let pad_style = Style::default().bg(th.panel);
    let mut fitted = crate::markdown::reflow_chat_lines_to_width(raw, panel_width);
    if fitted.len() > MAX_LIVE_STATUS_ROWS {
        let drop = fitted.len() - MAX_LIVE_STATUS_ROWS;
        fitted = fitted.split_off(drop);
    }
    crate::markdown::pad_lines_to_panel_width(fitted, panel_width, pad_style)
}

fn compose_chat_lines(
    th: ThemePalette,
    state: &AppState,
    entries: &[CachedMessageEntry],
    panel_width: u16,
) -> Vec<Line<'static>> {
    if state.chat_streaming.is_none() {
        reset_streaming_markdown_cache();
    }
    let mut lines = compose_history_lines(th, state, entries, panel_width);
    let live = compose_live_status_lines(th, state, panel_width);
    if live.is_empty() {
        return lines;
    }
    if !lines.is_empty() {
        lines.push(Line::from(""));
    }
    lines.extend(live);
    lines
}

struct ChatViewport {
    lines: Vec<Line<'static>>,
}

fn chat_viewport(th: ThemePalette, state: &AppState, width: u16) -> ChatViewport {
    let mut cache = RENDER_CACHE.lock().expect("chat render cache");
    let stale = cache.revision != state.chat_render_revision || cache.width != width;
    if stale {
        sync_message_entries(th, state, &mut cache.entries, width);
        cache.revision = state.chat_render_revision;
        cache.width = width;
    }
    ChatViewport {
        lines: compose_chat_lines(th, state, &cache.entries, width),
    }
}

/// Messages : Context width ratio when the context panel is open.
const MESSAGES_PANE_RATIO: u32 = 62;
const CONTEXT_PANE_RATIO: u32 = 38;

pub fn chat_pane_rects(area: Rect, context_visible: bool) -> (Rect, Option<Rect>) {
    if context_visible {
        let total = area.width.max(1);
        let messages_w = ((total as u32 * MESSAGES_PANE_RATIO)
            / (MESSAGES_PANE_RATIO + CONTEXT_PANE_RATIO))
            .max(1) as u16;
        let context_w = total.saturating_sub(messages_w).max(1);
        let messages = Rect {
            x: area.x,
            y: area.y,
            width: messages_w,
            height: area.height,
        };
        let context = Rect {
            x: area.x.saturating_add(messages_w),
            y: area.y,
            width: context_w,
            height: area.height,
        };
        (messages, Some(context))
    } else {
        (area, None)
    }
}

fn messages_format_width(inner_width: u16, context_visible: bool) -> u16 {
    let mut width = inner_width.max(1);
    if context_visible {
        // Reserve one column for the vertical scrollbar in split view.
        width = width.saturating_sub(1).max(1);
    }
    width
}

/// Map a click to Messages vs Context when the split is visible.
pub fn focus_pane_at(
    content_area: Rect,
    context_visible: bool,
    column: u16,
    row: u16,
) -> Option<coworker_core::app::ChatPaneFocus> {
    if !context_visible {
        return None;
    }
    let (messages, Some(context)) = chat_pane_rects(content_area, true) else {
        return None;
    };
    if context.contains(ratatui::layout::Position::new(column, row)) {
        return Some(coworker_core::app::ChatPaneFocus::Context);
    }
    if messages.contains(ratatui::layout::Position::new(column, row)) {
        return Some(coworker_core::app::ChatPaneFocus::Messages);
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
        || state.chat_pane_focus == coworker_core::app::ChatPaneFocus::Messages;
    let focus_mark = if state.chat_context_visible && messages_focused {
        " ◀"
    } else {
        ""
    };
    let history_block = theme::pane_block(
        th,
        format!("Messages{busy_suffix}{focus_mark}"),
        messages_focused,
    );
    let inner = history_block.inner(chunks[0]);
    let text_width = messages_format_width(inner.width, state.chat_context_visible);

    let vp = chat_viewport(th, state, text_width);
    let visible_height = inner.height.max(1);
    let total_lines = panel_line_count(&vp.lines, text_width);
    let max_scroll = total_lines.saturating_sub(visible_height);
    let scroll_from_bottom = state.chat_scroll_from_bottom.min(max_scroll);
    let scroll_y = max_scroll.saturating_sub(scroll_from_bottom);

    frame.render_widget(history_block, chunks[0]);
    frame.render_widget(Clear, inner);
    frame.render_widget(
        Paragraph::new(Text::from(vp.lines))
            .style(Style::default().bg(th.panel))
            .scroll((scroll_y, 0)),
        inner,
    );
    if total_lines > visible_height {
        let mut sb_state = paragraph_scrollbar_state(total_lines, visible_height, scroll_y);
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
            Span::styled(
                label,
                Style::default().fg(th.muted).add_modifier(Modifier::ITALIC),
            ),
        ])
    } else {
        Line::from(vec![
            Span::raw("  "),
            Span::styled(INPUT_PREFIX, Style::default().fg(th.accent)),
            Span::styled(state.chat_input.clone(), Style::default().fg(th.text)),
        ])
    };
    frame.render_widget(Paragraph::new(input_line).block(input_block), input_area);

    if focused {
        let cursor_x = input_inner
            .x
            .saturating_add((2 + INPUT_PREFIX.width() + state.chat_input.width()) as u16);
        frame.set_cursor_position(Position::new(cursor_x, input_inner.y));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reflow_splits_long_lines_to_panel_width() {
        use crate::markdown::reflow_chat_lines_to_width;
        let lines = vec![Line::from(
            "Actually I'll check ci_analyze_pr_failures(repo, pr_number) for other runs",
        )];
        let width = 28u16;
        let fitted = reflow_chat_lines_to_width(lines, width);
        assert!(fitted.len() > 1, "expected wrap into multiple rows");
        assert!(
            fitted
                .iter()
                .all(|line| crate::markdown::line_display_width_for_test(line) <= width as usize),
            "each row must fit split-pane width"
        );
    }

    #[test]
    fn live_status_fits_split_pane_width() {
        let th = ThemePalette::dark();
        let width = 30u16;
        let config = coworker_core::config::Config::load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../coworker.example.yaml"
        ))
        .expect("example config");
        let mut state = coworker_core::app::AppState::new(config, "test.yaml".into());
        state.chat_busy = true;
        state.chat_reasoning = Some(
            "Actually, I'll check if there are other runs for this PR. \
             Wait, looking at the tools: ci_analyze_pr_failures(repo, pr_number)"
                .into(),
        );
        let rows = compose_live_status_lines(th, &state, width);
        assert!(!rows.is_empty());
        assert!(
            rows.iter().all(|line| {
                crate::markdown::line_display_width_for_test(line) <= width as usize
            }),
            "live status must not emit over-wide rows"
        );
        assert!(rows.len() <= MAX_LIVE_STATUS_ROWS);
    }

    #[test]
    fn live_status_does_not_change_history_line_count() {
        let th = ThemePalette::dark();
        let width = 30u16;
        let entries = vec![CachedMessageEntry {
            source: "you> hello".into(),
            format_width: width,
            lines: theme::format_chat_lines(th, "you> hello", None),
        }];
        let config = coworker_core::config::Config::load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../coworker.example.yaml"
        ))
        .expect("example config");
        let mut state = coworker_core::app::AppState::new(config, "test.yaml".into());
        state.chat_busy = true;
        let short = compose_history_lines(th, &state, &entries, width);
        state.chat_reasoning = Some("short reasoning".into());
        let _live = compose_live_status_lines(th, &state, width);
        state.chat_reasoning = Some("much longer reasoning text that should only affect live status rows and not the scrollable history line count".into());
        let _live2 = compose_live_status_lines(th, &state, width);
        let long = compose_history_lines(th, &state, &entries, width);
        assert_eq!(
            short.len(),
            long.len(),
            "history scroll height must stay stable while reasoning streams"
        );
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
    fn reasoning_stream_tail_keeps_latest_chars() {
        let text = "alpha beta gamma delta epsilon";
        let tail = reasoning_stream_tail(text, 12);
        assert!(tail.starts_with('…'));
        assert!(tail.contains("epsilon"));
    }

    #[test]
    fn reasoning_preview_matches_thinking_header_style() {
        let th = ThemePalette::dark();
        let rows = reasoning_preview_lines(th, "Checking CI on PR #42", 60);
        assert!(!rows.is_empty());
        let header: String = rows[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(header.contains("reasoning"));
        assert!(header.contains('…'));
        assert!(rows.len() >= 2);
    }

    #[test]
    fn live_status_follows_history_in_scroll() {
        let th = ThemePalette::dark();
        let width = 40u16;
        let config = coworker_core::config::Config::load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../coworker.example.yaml"
        ))
        .expect("example config");

        type ChatRenderCase = (
            &'static str,
            Box<dyn FnOnce(&mut coworker_core::app::AppState)>,
        );
        let cases: Vec<ChatRenderCase> = vec![
            (
                "thinking",
                Box::new(|s| {
                    s.chat_busy = true;
                }),
            ),
            (
                "reasoning preview",
                Box::new(|s| {
                    s.chat_busy = true;
                    s.chat_reasoning = Some("Checking CI logs for PR #19194".into());
                }),
            ),
            (
                "reasoning summarizing",
                Box::new(|s| {
                    s.chat_busy = true;
                    s.chat_reasoning = Some("Long reasoning that will be summarized".into());
                    s.chat_reasoning_compressing = true;
                }),
            ),
            (
                "tool pending",
                Box::new(|s| {
                    s.chat_busy = true;
                    s.chat_tool_pending = Some("ci_get_failed_logs".into());
                }),
            ),
            (
                "tool running",
                Box::new(|s| {
                    s.chat_busy = true;
                    s.chat_tool_running = Some("ci_get_run_summary".into());
                }),
            ),
            (
                "streaming reply",
                Box::new(|s| {
                    s.chat_busy = true;
                    s.chat_streaming = Some("Partial assistant reply".into());
                }),
            ),
        ];

        let entries = vec![CachedMessageEntry {
            source: "you> hello".into(),
            format_width: width,
            lines: theme::format_chat_lines(th, "you> hello", None),
        }];

        for (label, setup) in cases {
            let mut state = coworker_core::app::AppState::new(config.clone(), "test.yaml".into());
            setup(&mut state);
            let rows = compose_chat_lines(th, &state, &entries, width);
            let history_only = compose_history_lines(th, &state, &entries, width);
            assert!(
                rows.len() > history_only.len(),
                "{label}: live status should extend the scrollable transcript"
            );
            let activity_idx = history_only.len() + 1;
            let activity_header: String = rows
                .get(activity_idx)
                .map(|line| line.spans.iter().map(|s| s.content.as_ref()).collect())
                .unwrap_or_default();
            assert!(
                activity_header.contains('…')
                    || activity_header.contains("waiting")
                    || activity_header.contains("running")
                    || activity_header.contains("streaming")
                    || activity_header.contains("reasoning")
                    || activity_header.contains("summarizing")
                    || activity_header.contains('◔'),
                "{label}: expected live status row after history spacer, got {activity_header:?}"
            );
        }
    }

    #[test]
    fn long_reasoning_live_status_is_capped() {
        let th = ThemePalette::dark();
        let width = 28u16;
        let config = coworker_core::config::Config::load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../coworker.example.yaml"
        ))
        .expect("example config");
        let mut state = coworker_core::app::AppState::new(config, "test.yaml".into());
        state.chat_busy = true;
        state.chat_reasoning = Some(
            "Actually I'll check if there are other runs for this PR. \
            ci_analyze_pr_failures(repo, pr_number) "
                .repeat(8),
        );
        let rows = compose_live_status_lines(th, &state, width);
        assert!(
            rows.len() <= MAX_LIVE_STATUS_ROWS,
            "live status should be capped, got {}",
            rows.len()
        );
    }

    #[test]
    fn reasoning_live_status_scroll_height_is_bounded() {
        let th = ThemePalette::dark();
        let width = 28u16;
        let config = coworker_core::config::Config::load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../coworker.example.yaml"
        ))
        .expect("example config");
        let mut state = coworker_core::app::AppState::new(config, "test.yaml".into());
        state.chat_busy = true;
        state.chat_reasoning = Some(
            "Actually I'll check if there are other runs for this PR. \
            ci_analyze_pr_failures(repo, pr_number) "
                .repeat(8),
        );
        let rows = compose_live_status_lines(th, &state, width);
        assert!(
            rows.len() <= MAX_LIVE_STATUS_ROWS,
            "live status should be capped, got {}",
            rows.len()
        );
    }

    #[test]
    fn reasoning_body_caps_logical_source_lines() {
        let th = ThemePalette::dark();
        let width = 28u16;
        let text = "line one\nline two\nline three\nline four\nline five\nline six\nline seven\nline eight\nline nine"
            .to_string();
        let rows = reasoning_body_lines(th, &text, width);
        assert!(
            rows.len() <= REASONING_TAIL_MAX_ROWS,
            "expected <= {REASONING_TAIL_MAX_ROWS} logical rows, got {}",
            rows.len()
        );
    }

    #[test]
    fn skips_reasoning_transcript_lines() {
        let entry = CachedMessageEntry {
            source: "  … reasoning: Checked CI on PR #42".into(),
            format_width: 0,
            lines: vec![Line::from("hidden")],
        };
        assert!(should_skip_reasoning_transcript(&entry));
    }

    #[test]
    fn scrollbar_max_scroll_at_bottom_pin() {
        let raw = vec![
            Line::from("Hello world this is a long line that should wrap across cells"),
            Line::from("short"),
        ];
        let width = 12u16;
        let visible = 3u16;
        let lines = crate::markdown::reflow_chat_lines_to_width(raw, width);
        let total = panel_line_count(&lines, width);
        let max_scroll = total.saturating_sub(visible);
        let scroll_y = max_scroll;
        assert_eq!(scroll_y as usize + visible as usize, total as usize);
    }

    #[test]
    fn skips_duplicate_tool_transcript_after_tool_row() {
        let entries = vec![
            CachedMessageEntry {
                source: "  ✓ pr_list_changed_files(repo=acme/widget, pr_number=19275) (120ms)".into(),
                format_width: 0,
                lines: vec![],
            },
            CachedMessageEntry {
                source: "assistant> tool_result(pr_list_changed_files, pr_number=19275):\n1 changed file(s)"
                    .into(),
                format_width: 0,
                lines: vec![],
            },
        ];
        assert!(should_skip_tool_transcript_echo(&entries, 1));
        assert!(!should_skip_tool_transcript_echo(&entries, 0));
    }

    #[test]
    fn assistant_message_table_rows_stay_single_line() {
        let th = ThemePalette::dark();
        let width = 48u16;
        let max_width = width.saturating_sub(8) as usize;
        let body = "| PR | CI | Review |\n|----|----|--------|\n| #19274 | failing | pending |\n| #19273 | ok | approved |";
        let rows = theme::format_assistant_message_lines(th, body, Some(max_width));
        let fitted = crate::markdown::ensure_chat_lines_fit_panel(rows, width);
        let table_line_count = fitted
            .iter()
            .filter(|l| {
                let text: String = l.spans.iter().map(|s| s.content.as_ref()).collect();
                let t = text.trim();
                t.starts_with('│') || (t.starts_with('├') && t.ends_with('┤'))
            })
            .count();
        assert!(table_line_count >= 2, "expected formatted table rows");
        assert!(
            fitted.iter().all(|line| {
                crate::markdown::line_display_width_for_test(line) <= width as usize
            }),
            "table rows should fit panel without Paragraph re-wrap"
        );
    }

    #[test]
    fn assistant_long_markdown_no_vertical_glyph_bleed() {
        let th = ThemePalette::dark();
        let width = 42u16;
        let body = "## 4. How We Work Together\n\n* **Enhancements** — iterative workflow.\n* **Productivity** — use read_file before edit_file.";
        let rows = theme::format_assistant_message_lines(
            th,
            body,
            Some(theme::chat_content_max_width(width)),
        );
        let fitted = crate::markdown::pad_lines_to_panel_width(
            crate::markdown::ensure_chat_lines_fit_panel(rows, width),
            width,
            Style::default(),
        );
        assert!(fitted.len() > 2);
        for line in &fitted {
            let w = crate::markdown::line_display_width_for_test(line);
            assert_eq!(
                w, width as usize,
                "padded row must fill panel: {w} != {width}: {line:?}"
            );
            let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            assert!(
                !text.starts_with('▌') || text.contains("AI") || text.trim() == "▌",
                "orphan bar glyph column: {text:?}"
            );
        }
    }

    #[test]
    fn go_code_block_fits_panel_without_horizontal_bleed() {
        let th = ThemePalette::dark();
        let width = 50u16;
        let body = "4. Create routes/user.go\n\n```go\nuserRoutes := router.Group(\"/users\")\nuserRoutes.GET(\"/\", handlers.GetUsers)\nuserRoutes.POST(\"/\", handlers.CreateUser)\n```";
        let rows = theme::format_assistant_message_lines(
            th,
            body,
            Some(theme::chat_content_max_width(width)),
        );
        let fitted = crate::markdown::pad_lines_to_panel_width(
            crate::markdown::ensure_chat_lines_fit_panel(rows, width),
            width,
            Style::default(),
        );
        assert!(
            fitted.iter().all(|line| {
                crate::markdown::line_display_width_for_test(line) == width as usize
            }),
            "every row must be exactly panel width to avoid ghost columns"
        );
    }

    #[test]
    fn system_session_header_fits_panel() {
        let th = ThemePalette::dark();
        let width = 48u16;
        let rows = theme::format_chat_lines(
            th,
            "system> recent sessions (use /session <id-prefix>):",
            Some(theme::chat_content_max_width(width)),
        );
        let fitted = crate::markdown::pad_lines_to_panel_width(
            crate::markdown::ensure_chat_lines_fit_panel(rows, width),
            width,
            Style::default(),
        );
        assert!(fitted
            .iter()
            .all(|line| { crate::markdown::line_display_width_for_test(line) == width as usize }));
    }

    #[test]
    fn focus_pane_at_respects_split() {
        use coworker_core::app::ChatPaneFocus;
        let area = Rect::new(0, 5, 100, 20);
        let (messages, context) = chat_pane_rects(area, true);
        let context = context.expect("context pane");
        assert_eq!(messages.width + context.width, area.width);
        assert!(
            messages.width >= 60 && context.width >= 35,
            "expected ~62/38 split, got messages={} context={}",
            messages.width,
            context.width
        );
        assert_eq!(
            focus_pane_at(area, true, messages.x + 2, messages.y + 2),
            Some(ChatPaneFocus::Messages)
        );
        assert_eq!(
            focus_pane_at(area, true, context.x + 2, context.y + 2),
            Some(ChatPaneFocus::Context)
        );
        assert_eq!(
            focus_pane_at(area, false, messages.x + 2, messages.y + 2),
            None
        );
    }

    #[test]
    fn consecutive_tool_rows_group_without_blank_gap() {
        let th = ThemePalette::dark();
        let width = 60u16;
        let entries = vec![
            CachedMessageEntry {
                source: "you> list PRs".into(),
                format_width: width,
                lines: theme::format_chat_lines(th, "you> list PRs", None),
            },
            CachedMessageEntry {
                source: "  ✓ pr_list_open(repo=acme/widget)".into(),
                format_width: width,
                lines: theme::format_chat_lines(th, "  ✓ pr_list_open(repo=acme/widget)", None),
            },
            CachedMessageEntry {
                source: "  ✓ pr_get_overview(repo=acme/widget, pr=42)".into(),
                format_width: width,
                lines: theme::format_chat_lines(
                    th,
                    "  ✓ pr_get_overview(repo=acme/widget, pr=42)",
                    None,
                ),
            },
        ];
        let config = coworker_core::config::Config::load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../coworker.example.yaml"
        ))
        .expect("example config");
        let state = coworker_core::app::AppState::new(config, "test.yaml".into());
        let rows = compose_history_lines(th, &state, &entries, width);
        let joined: Vec<String> = rows
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect();
        let first_tool = joined
            .iter()
            .position(|l| l.contains("pr_list_open"))
            .expect("first tool row");
        let second_tool = joined
            .iter()
            .position(|l| l.contains("pr_get_overview"))
            .expect("second tool row");
        assert_eq!(
            second_tool,
            first_tool + 1,
            "tool rows in a group should not have blank spacer: {joined:?}"
        );
        assert!(
            joined[first_tool].starts_with("      │ "),
            "first tool row should use group connector: {:?}",
            joined[first_tool]
        );
        assert!(
            joined[second_tool].starts_with("      │ "),
            "second tool row should use group connector: {:?}",
            joined[second_tool]
        );
    }

    #[test]
    fn tool_running_line_shows_progress_detail() {
        let th = ThemePalette::dark();
        let rows = tool_running_lines(th, "ci_get_failed_logs", Some("page 2, 5s"));
        let text: String = rows[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("ci_get_failed_logs"));
        assert!(text.contains("page 2, 5s"));
    }
}
