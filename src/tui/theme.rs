use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders};
use unicode_width::UnicodeWidthStr;

use crate::config::TuiThemeMode;

/// Full TUI color palette (dark or light).
#[derive(Debug, Clone, Copy)]
pub struct ThemePalette {
    pub bg: Color,
    pub surface: Color,
    pub panel: Color,
    pub input_bg: Color,
    pub border: Color,
    pub border_active: Color,
    pub accent: Color,
    pub accent_dim: Color,
    pub muted: Color,
    pub text: Color,
    pub user: Color,
    pub user_bg: Color,
    pub assistant: Color,
    pub ai_bg: Color,
    pub tool: Color,
    pub ok: Color,
    pub err: Color,
    pub warn: Color,
    pub title_bg: Color,
    pub tab_active_bg: Color,
    pub badge_fg: Color,
    pub link: Color,
    pub code_fg: Color,
    pub code_bg: Color,
    pub pr_ref: Color,
    pub heading_h1: Color,
    pub heading_h2: Color,
}

impl ThemePalette {
    pub fn from_mode(mode: TuiThemeMode) -> Self {
        match mode {
            TuiThemeMode::Dark => Self::dark(),
            TuiThemeMode::Light => Self::light(),
        }
    }

    /// Catppuccin-inspired dark (default).
    pub fn dark() -> Self {
        Self {
            bg: Color::Rgb(24, 26, 32),
            surface: Color::Rgb(30, 32, 40),
            panel: Color::Rgb(36, 39, 48),
            input_bg: Color::Rgb(28, 30, 38),
            border: Color::Rgb(52, 55, 68),
            border_active: Color::Rgb(72, 125, 165),
            accent: Color::Rgb(137, 180, 250),
            accent_dim: Color::Rgb(88, 130, 185),
            muted: Color::Rgb(108, 112, 128),
            text: Color::Rgb(205, 214, 244),
            user: Color::Rgb(166, 227, 161),
            user_bg: Color::Rgb(64, 120, 70),
            assistant: Color::Rgb(180, 190, 220),
            ai_bg: Color::Rgb(70, 100, 150),
            tool: Color::Rgb(249, 226, 175),
            ok: Color::Rgb(166, 227, 161),
            err: Color::Rgb(243, 139, 168),
            warn: Color::Rgb(250, 179, 135),
            title_bg: Color::Rgb(42, 45, 56),
            tab_active_bg: Color::Rgb(55, 70, 95),
            badge_fg: Color::Rgb(24, 26, 32),
            link: Color::Rgb(147, 197, 253),
            code_fg: Color::Rgb(250, 220, 160),
            code_bg: Color::Rgb(48, 52, 64),
            pr_ref: Color::Rgb(250, 179, 135),
            heading_h1: Color::Rgb(137, 180, 250),
            heading_h2: Color::Rgb(166, 200, 240),
        }
    }

    /// Clean light theme for bright terminals.
    pub fn light() -> Self {
        Self {
            bg: Color::Rgb(245, 247, 250),
            surface: Color::Rgb(255, 255, 255),
            panel: Color::Rgb(252, 252, 254),
            input_bg: Color::Rgb(255, 255, 255),
            border: Color::Rgb(210, 215, 225),
            border_active: Color::Rgb(59, 130, 246),
            accent: Color::Rgb(37, 99, 235),
            accent_dim: Color::Rgb(59, 130, 196),
            muted: Color::Rgb(100, 116, 139),
            text: Color::Rgb(30, 41, 59),
            user: Color::Rgb(21, 128, 61),
            user_bg: Color::Rgb(134, 239, 172),
            assistant: Color::Rgb(51, 65, 85),
            ai_bg: Color::Rgb(147, 197, 253),
            tool: Color::Rgb(180, 83, 9),
            ok: Color::Rgb(22, 163, 74),
            err: Color::Rgb(220, 38, 38),
            warn: Color::Rgb(234, 88, 12),
            title_bg: Color::Rgb(241, 245, 249),
            tab_active_bg: Color::Rgb(219, 234, 254),
            badge_fg: Color::Rgb(255, 255, 255),
            link: Color::Rgb(37, 99, 235),
            code_fg: Color::Rgb(154, 52, 18),
            code_bg: Color::Rgb(241, 245, 249),
            pr_ref: Color::Rgb(194, 65, 12),
            heading_h1: Color::Rgb(30, 64, 175),
            heading_h2: Color::Rgb(29, 78, 216),
        }
    }
}

pub fn frame_block(th: ThemePalette, title: impl Into<String>) -> Block<'static> {
    pane_block(th, title, false)
}

pub fn pane_block(th: ThemePalette, title: impl Into<String>, focused: bool) -> Block<'static> {
    let border = if focused { th.border_active } else { th.border };
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border))
        .style(Style::default().bg(th.panel))
        .title(Span::styled(
            format!(" {} ", title.into()),
            Style::default()
                .fg(if focused { th.accent } else { th.muted })
                .bg(th.panel)
                .add_modifier(Modifier::BOLD),
        ))
        .title_bottom(Span::raw(""))
}

pub fn input_block(th: ThemePalette, title: impl Into<String>, focused: bool) -> Block<'static> {
    let border = if focused { th.border_active } else { th.border };
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border))
        .style(Style::default().bg(th.input_bg))
        .title(Span::styled(
            format!(" {} ", title.into()),
            Style::default()
                .fg(if focused { th.accent } else { th.muted })
                .bg(th.input_bg)
                .add_modifier(Modifier::BOLD),
        ))
}

pub fn list_block(th: ThemePalette, title: &str) -> Block<'static> {
    frame_block(th, title)
}

pub fn detail_block(th: ThemePalette) -> Block<'static> {
    frame_block(th, "Detail")
}

pub fn header_block(th: ThemePalette) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(th.border))
        .style(Style::default().bg(th.surface))
        .title(Span::styled(
            " unistar-coworker ",
            Style::default()
                .fg(th.accent)
                .bg(th.surface)
                .add_modifier(Modifier::BOLD),
        ))
        .title_alignment(ratatui::layout::Alignment::Left)
}

pub fn hint_bar(th: ThemePalette, text: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(" ", Style::default().bg(th.title_bg)),
        Span::styled(
            text.to_string(),
            Style::default().fg(th.muted).bg(th.title_bg),
        ),
        Span::styled(" ", Style::default().bg(th.title_bg)),
    ])
}

pub fn status_line(
    th: ThemePalette,
    busy: bool,
    status: &str,
    mcp_ok: bool,
    llm_ok: bool,
    alert_note: &str,
) -> Line<'static> {
    let dot = if busy { th.warn } else { th.ok };
    Line::from(vec![
        Span::styled(" ", Style::default().bg(th.surface)),
        Span::styled("●", Style::default().fg(dot).bg(th.surface)),
        Span::styled(
            format!(" {}", if busy { "busy" } else { "idle" }),
            Style::default().fg(th.muted).bg(th.surface),
        ),
        Span::styled(" │ ", Style::default().fg(th.border).bg(th.surface)),
        Span::styled(
            status.to_string(),
            Style::default().fg(th.text).bg(th.surface),
        ),
        Span::styled(" │ mcp ", Style::default().fg(th.border).bg(th.surface)),
        Span::styled(
            if mcp_ok { "ok" } else { "off" },
            Style::default()
                .fg(if mcp_ok { th.ok } else { th.err })
                .bg(th.surface),
        ),
        Span::styled(" │ llm ", Style::default().fg(th.border).bg(th.surface)),
        Span::styled(
            if llm_ok { "ok" } else { "off" },
            Style::default()
                .fg(if llm_ok { th.ok } else { th.err })
                .bg(th.surface),
        ),
        Span::styled(
            alert_note.to_string(),
            Style::default().fg(th.warn).bg(th.surface),
        ),
        Span::styled(" ", Style::default().bg(th.surface)),
    ])
}

pub fn tab_spans(th: ThemePalette, label: &str, active: bool) -> Span<'static> {
    if active {
        Span::styled(
            format!(" {label} "),
            Style::default()
                .fg(th.accent)
                .bg(th.tab_active_bg)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(
            format!(" {label} "),
            Style::default().fg(th.muted).bg(th.surface),
        )
    }
}

pub fn tab_separator(th: ThemePalette) -> Span<'static> {
    Span::styled("│", Style::default().fg(th.border).bg(th.surface))
}

/// Usable markdown width inside the Messages pane after the continuation indent.
pub fn message_content_max_width(panel_width: u16) -> usize {
    content_max_width(panel_width, message_indent_width())
}

/// Usable markdown width for the streaming tail body (deeper indent).
pub fn tail_content_max_width(panel_width: u16) -> usize {
    content_max_width(panel_width, tail_body_indent().len())
}

fn content_max_width(panel_width: u16, indent_cols: usize) -> usize {
    panel_width
        .saturating_sub(indent_cols as u16)
        .max(1) as usize
}

/// Expand one stored chat row into styled terminal lines.
pub fn format_chat_lines(
    th: ThemePalette,
    line: &str,
    max_width: Option<usize>,
) -> Vec<Line<'static>> {
    if let Some(rest) = line.strip_prefix("you> ") {
        return message_lines(th, "You", th.user, th.user_bg, rest, false, max_width);
    }
    if let Some(rest) = line.strip_prefix("assistant> ") {
        if crate::agent::context::is_tool_result_transcript(rest) {
            return format_tool_transcript_lines(th, rest, max_width);
        }
        return format_assistant_message_lines(th, rest, max_width);
    }
    if let Some(rest) = line.strip_prefix("  ⚠ ") {
        return vec![Line::from(vec![
            Span::raw("      "),
            Span::styled("⚠ ", Style::default().fg(th.warn)),
            Span::styled(
                rest.to_string(),
                Style::default()
                    .fg(th.warn)
                    .add_modifier(Modifier::ITALIC),
            ),
        ])];
    }
    if let Some(rest) = line.strip_prefix("  → ") {
        return vec![Line::from(vec![
            Span::raw("      "),
            Span::styled("◔ ", Style::default().fg(th.tool)),
            Span::styled(
                rest.to_string(),
                Style::default().fg(th.tool).add_modifier(Modifier::ITALIC),
            ),
        ])];
    }
    if let Some(rest) = line.strip_prefix("  ✓ ") {
        return vec![Line::from(vec![
            Span::raw("      "),
            Span::styled("✓ ", Style::default().fg(th.ok)),
            Span::styled(rest.to_string(), Style::default().fg(th.muted)),
        ])];
    }
    if let Some(rest) = line.strip_prefix("  ✗ ") {
        return vec![Line::from(vec![
            Span::raw("      "),
            Span::styled("✗ ", Style::default().fg(th.err)),
            Span::styled(rest.to_string(), Style::default().fg(th.err)),
        ])];
    }
    if let Some(rest) = line.strip_prefix("error> ") {
        return vec![Line::from(vec![
            Span::raw("      "),
            Span::styled("⚠ ", Style::default().fg(th.err)),
            Span::styled(rest.to_string(), Style::default().fg(th.err)),
        ])];
    }
    line.split('\n')
        .map(|part| Line::from(Span::styled(part.to_string(), Style::default().fg(th.text))))
        .collect()
}

/// Styled lines for an assistant message body (same path for streaming and final render).
pub fn format_assistant_message_lines(
    th: ThemePalette,
    body: &str,
    max_width: Option<usize>,
) -> Vec<Line<'static>> {
    let body = normalize_message_layout(body);
    message_lines(th, "AI", th.accent, th.ai_bg, &body, true, max_width)
}

/// Assistant reply body for the tail status area (indented under a spinner header).
pub fn format_assistant_tail_body_lines(
    th: ThemePalette,
    body: &str,
    max_width: Option<usize>,
) -> Vec<Line<'static>> {
    let body = normalize_message_layout(body);
    let content_style = Style::default().fg(th.assistant);
    let content_lines =
        super::markdown::markdown_to_lines_in_width(th, &body, content_style, max_width);
    let indent = tail_body_indent();
    content_lines
        .into_iter()
        .map(|content| {
            if content.spans.is_empty() {
                Line::from("")
            } else {
                let mut spans = vec![Span::raw(indent.clone())];
                spans.extend(content.spans);
                Line::from(spans)
            }
        })
        .collect()
}

fn tail_body_indent() -> String {
    "        ".to_string()
}
pub fn normalize_message_layout(body: &str) -> String {
    let body = body.replace("\r\n", "\n");
    if body.contains('\n') {
        return body;
    }
    let mut s = body;
    for (from, to) in [
        (")* **", ")\n* **"),
        (" * **#", "\n* **#"),
        (":* **#", ":\n* **#"),
        (" * - ", "\n* - "),
    ] {
        if s.contains(from) {
            s = s.replace(from, to);
        }
    }
    s
}

fn message_lines(
    th: ThemePalette,
    role: &str,
    accent: Color,
    badge_bg: Color,
    body: &str,
    markdown: bool,
    max_width: Option<usize>,
) -> Vec<Line<'static>> {
    let content_style = Style::default().fg(if role == "You" { th.user } else { th.assistant });
    let content_lines = if markdown {
        super::markdown::markdown_to_lines_in_width(th, body, content_style, max_width)
    } else {
        body.split('\n')
            .map(|part| {
                if part.is_empty() {
                    Line::from("")
                } else {
                    Line::from(Span::styled(part.to_string(), content_style))
                }
            })
            .collect()
    };

    let indent = " ".repeat(message_indent_width());
    let bar = Span::styled("▌ ", Style::default().fg(accent));
    let badge = role_badge(th, role, badge_bg);

    let mut out = Vec::with_capacity(content_lines.len().max(1));
    for (i, content) in content_lines.into_iter().enumerate() {
        if i == 0 {
            let mut spans = vec![bar.clone(), badge.clone()];
            spans.extend(content.spans);
            out.push(Line::from(spans));
        } else if content.spans.is_empty() {
            out.push(Line::from(""));
        } else {
            let mut spans = vec![Span::raw(indent.clone())];
            spans.extend(content.spans);
            out.push(Line::from(spans));
        }
    }
    if out.is_empty() {
        out.push(Line::from(vec![bar, badge]));
    }
    out
}

fn role_badge(th: ThemePalette, role: &str, bg: Color) -> Span<'static> {
    Span::styled(
        format!(" {role} "),
        Style::default()
            .fg(th.badge_fg)
            .bg(bg)
            .add_modifier(Modifier::BOLD),
    )
}

fn message_indent_width() -> usize {
    "▌  You ".width()
}

pub fn ci_status_style(th: ThemePalette, summary: &str) -> Style {
    let lower = summary.to_ascii_lowercase();
    if lower.contains("fail") || lower.contains("red") {
        Style::default().fg(th.err)
    } else if lower.contains("ok") || lower.contains("green") || lower.contains("pass") {
        Style::default().fg(th.ok)
    } else if lower.contains("pending") || lower.contains("wait") {
        Style::default().fg(th.warn)
    } else {
        Style::default().fg(th.muted)
    }
}

/// Compact CI status glyph for PR list rows.
pub fn pr_ci_glyph(summary: &str) -> &'static str {
    let lower = summary.to_ascii_lowercase();
    if lower.contains("fail") || lower.contains("red") {
        "✗"
    } else if lower.contains("ok") || lower.contains("green") || lower.contains("pass") {
        "✓"
    } else if lower.contains("pending") || lower.contains("wait") {
        "◷"
    } else {
        "·"
    }
}

/// Compact review status glyph for PR list rows.
pub fn pr_review_glyph(summary: &str) -> &'static str {
    let lower = summary.to_ascii_lowercase();
    if lower.contains("changes") {
        "✗"
    } else if lower.contains("review") {
        "◉"
    } else if lower.contains("approved") {
        "✓"
    } else {
        "·"
    }
}

pub fn review_status_style(th: ThemePalette, summary: &str) -> Style {
    let lower = summary.to_ascii_lowercase();
    if lower.contains("changes") {
        Style::default().fg(th.err)
    } else if lower.contains("review") {
        Style::default().fg(th.warn)
    } else if lower.contains("approved") {
        Style::default().fg(th.ok)
    } else {
        Style::default().fg(th.muted)
    }
}

pub fn log_level_style(th: ThemePalette, level: &str) -> Style {
    match level.to_ascii_lowercase().as_str() {
        "error" => Style::default().fg(th.err).add_modifier(Modifier::BOLD),
        "warn" | "warning" => Style::default().fg(th.warn),
        "info" => Style::default().fg(th.accent_dim),
        "debug" => Style::default().fg(th.muted),
        _ => Style::default().fg(th.text),
    }
}

/// Indented tool output lines shown when a completed tool row is expanded.
pub fn format_tool_detail_lines(th: ThemePalette, body: &str) -> Vec<Line<'static>> {
    const MAX_LINES: usize = 80;
    let style = Style::default().fg(th.muted);
    body.lines()
        .take(MAX_LINES)
        .map(|line| {
            Line::from(vec![
                Span::raw("        "),
                Span::styled(line.to_string(), style),
            ])
        })
        .collect()
}

/// Render a mistaken `assistant>` tool transcript as tool output (not an AI reply).
pub fn format_tool_transcript_lines(
    th: ThemePalette,
    transcript: &str,
    max_width: Option<usize>,
) -> Vec<Line<'static>> {
    let Some((name, body)) = crate::agent::context::split_tool_transcript(transcript) else {
        return format_tool_detail_lines(th, transcript);
    };
    let mut out = vec![Line::from(vec![
        Span::raw("      "),
        Span::styled("✓ ", Style::default().fg(th.ok)),
        Span::styled(name, Style::default().fg(th.muted)),
        Span::styled(" (transcript)", Style::default().fg(th.muted).add_modifier(Modifier::ITALIC)),
    ])];
    if body.is_empty() {
        return out;
    }
    let detail = if transcript.contains('|') && transcript.contains("---") {
        super::markdown::markdown_to_lines_in_width(
            th,
            &body,
            Style::default().fg(th.muted),
            max_width,
        )
        .into_iter()
        .map(|line| {
            if line.spans.is_empty() {
                Line::from("")
            } else {
                let mut spans = vec![Span::raw("        ")];
                spans.extend(line.spans);
                Line::from(spans)
            }
        })
        .collect()
    } else {
        format_tool_detail_lines(th, &body)
    };
    out.extend(detail);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assistant_tail_body_uses_same_markdown_as_message() {
        let th = ThemePalette::dark();
        let body = "* **#1**: first item\n* **#2**: second";
        let message = format_assistant_message_lines(th, body, None);
        let tail = format_assistant_tail_body_lines(th, body, None);
        assert_eq!(message.len(), tail.len());
        let msg_text: Vec<String> = message
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect();
        let tail_text: Vec<String> = tail
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect();
        for (m, t) in msg_text.iter().zip(tail_text.iter()) {
            assert!(
                t.trim() == m.trim() || m.contains(t.trim()),
                "tail {t:?} should match message body in {m:?}"
            );
        }
    }

    #[test]
    fn assistant_tool_transcript_uses_tool_style_not_ai_badge() {
        let th = ThemePalette::dark();
        let raw = "assistant> tool_result(pr_list_changed_files, pr_number=19275):\n1 changed file(s)";
        let rows = format_chat_lines(th, raw, None);
        let joined: String = rows
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.as_ref())
            .collect();
        assert!(joined.contains('✓'));
        assert!(joined.contains("pr_list_changed_files"));
        assert!(!joined.contains(" AI "));
    }

    #[test]
    fn multiline_assistant_splits() {
        let th = ThemePalette::dark();
        let rows = format_chat_lines(th, "assistant> line one\nline two", None);
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn normalizes_crammed_bullets() {
        let raw = "PRs in repo:* **#1**: foo (CI: ok)* **#2**: bar";
        let norm = normalize_message_layout(raw);
        assert!(norm.contains('\n'));
        assert!(norm.contains("* **#2**"));
    }

    #[test]
    fn assistant_markdown_bold() {
        let th = ThemePalette::dark();
        let rows = format_chat_lines(th, "assistant> **hello** world", None);
        assert!(!rows.is_empty());
        assert!(rows[0]
            .spans
            .iter()
            .any(|s| s.style.add_modifier.contains(Modifier::BOLD)));
    }

    #[test]
    fn tool_lines_use_icons() {
        let th = ThemePalette::dark();
        let row = format_chat_lines(th, "  → pr_list_open(repo=x)", None)[0].clone();
        let text: String = row.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains('◔'));
    }

    #[test]
    fn duplicate_tool_line_uses_warn_style() {
        let th = ThemePalette::dark();
        let row = format_chat_lines(
            th,
            "  ⚠ duplicate pr_get_overview(repo=x, pr=1) (attempt 2)",
            None,
        )[0]
        .clone();
        assert!(row
            .spans
            .iter()
            .any(|s| s.content.contains('⚠') && s.style.fg == Some(th.warn)));
    }

    #[test]
    fn light_palette_differs_from_dark() {
        assert_ne!(ThemePalette::dark().bg, ThemePalette::light().bg);
        assert_ne!(ThemePalette::dark().text, ThemePalette::light().text);
    }

    #[test]
    fn pr_glyphs_reflect_status() {
        assert_eq!(pr_ci_glyph("passing (3/3)"), "✓");
        assert_eq!(pr_ci_glyph("failing (1/3)"), "✗");
        assert_eq!(pr_review_glyph("review-required"), "◉");
        assert_eq!(pr_review_glyph("approved"), "✓");
    }

    #[test]
    fn tool_detail_lines_indent() {
        let th = ThemePalette::dark();
        let rows = format_tool_detail_lines(th, "line one\nline two");
        assert_eq!(rows.len(), 2);
        assert!(rows[0].spans[0].content.starts_with("        "));
    }
}
