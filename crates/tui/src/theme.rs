use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders};
use unicode_width::UnicodeWidthStr;

use coworker_core::config::{ThemeMode, TuiConfig};

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
    pub heading_h3: Color,
    pub heading_h4: Color,
    /// When true, markdown links use OSC 8 escape sequences (modern terminals).
    pub osc8_links: bool,
}

impl ThemePalette {
    pub fn from_mode(mode: ThemeMode) -> Self {
        match mode {
            ThemeMode::Dark => Self::dark(),
            ThemeMode::Light => Self::light(),
            ThemeMode::None => Self::none(),
        }
    }

    pub fn from_tui(tui: &TuiConfig, theme: ThemeMode) -> Self {
        let mut palette = Self::from_mode(theme);
        palette.osc8_links = tui.osc8_links;
        if theme != ThemeMode::None {
            if let Some(accent) = tui.accent.as_deref().and_then(parse_hex_rgb) {
                palette.accent = accent;
                palette.accent_dim = dim_rgb(accent);
                palette.link = accent;
                palette.heading_h2 = accent;
            }
        }
        palette
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
            heading_h1: Color::Rgb(205, 214, 244),
            heading_h2: Color::Rgb(137, 180, 250),
            heading_h3: Color::Rgb(250, 179, 135),
            heading_h4: Color::Rgb(166, 200, 240),
            osc8_links: false,
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
            heading_h1: Color::Rgb(15, 23, 42),
            heading_h2: Color::Rgb(30, 64, 175),
            heading_h3: Color::Rgb(194, 65, 12),
            heading_h4: Color::Rgb(29, 78, 216),
            osc8_links: false,
        }
    }

    /// Minimal styling — respects the terminal's own color scheme.
    pub fn none() -> Self {
        Self {
            bg: Color::Reset,
            surface: Color::Reset,
            panel: Color::Reset,
            input_bg: Color::Reset,
            border: Color::Reset,
            border_active: Color::Cyan,
            accent: Color::Cyan,
            accent_dim: Color::DarkGray,
            muted: Color::DarkGray,
            text: Color::Reset,
            user: Color::Green,
            user_bg: Color::Reset,
            assistant: Color::Reset,
            ai_bg: Color::Reset,
            tool: Color::Yellow,
            ok: Color::Green,
            err: Color::Red,
            warn: Color::Yellow,
            title_bg: Color::Reset,
            tab_active_bg: Color::Reset,
            badge_fg: Color::Reset,
            link: Color::Cyan,
            code_fg: Color::Yellow,
            code_bg: Color::Reset,
            pr_ref: Color::Magenta,
            heading_h1: Color::Reset,
            heading_h2: Color::Cyan,
            heading_h3: Color::Yellow,
            heading_h4: Color::Blue,
            osc8_links: false,
        }
    }
}

fn parse_hex_rgb(hex: &str) -> Option<Color> {
    let s = hex.trim().trim_start_matches('#');
    if s.len() != 6 || !s.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let r = u8::from_str_radix(&s[0..2], 16).ok()?;
    let g = u8::from_str_radix(&s[2..4], 16).ok()?;
    let b = u8::from_str_radix(&s[4..6], 16).ok()?;
    Some(Color::Rgb(r, g, b))
}

fn dim_rgb(c: Color) -> Color {
    match c {
        Color::Rgb(r, g, b) => Color::Rgb(
            (u16::from(r) * 2 / 3) as u8,
            (u16::from(g) * 2 / 3) as u8,
            (u16::from(b) * 2 / 3) as u8,
        ),
        other => other,
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
    github_ok: bool,
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
        Span::styled(" │ gh ", Style::default().fg(th.border).bg(th.surface)),
        Span::styled(
            if github_ok { "ok" } else { "opt" },
            Style::default()
                .fg(if github_ok { th.ok } else { th.muted })
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

/// Map a column (relative to header inner left) to a tab label hit target.
pub fn tab_at_column(
    tabs: &[coworker_core::app::Tab],
    rel_x: usize,
) -> Option<coworker_core::app::Tab> {
    let mut x = 0usize;
    for (i, tab) in tabs.iter().enumerate() {
        if i > 0 {
            x += UnicodeWidthStr::width("│");
        }
        let label = format!(" {} ", tab.label());
        let w = UnicodeWidthStr::width(label.as_str());
        if rel_x < x + w {
            return Some(*tab);
        }
        x += w;
    }
    None
}

pub fn header_inner_area(full: Rect) -> Rect {
    let header = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(full)[0];
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(" unistar-coworker ")
        .inner(header)
}

/// Usable markdown width inside the Messages pane (continuation indent).
pub fn chat_content_max_width(panel_width: u16) -> usize {
    content_max_width(panel_width, message_indent_width())
}

/// Usable body width inside the Context panel (`  ` prefix per line).
pub fn context_content_max_width(panel_width: u16) -> usize {
    content_max_width(panel_width, 2)
}

/// Usable markdown width for the streaming tail body (deeper indent).
pub fn tail_content_max_width(panel_width: u16) -> usize {
    content_max_width(panel_width, tail_body_indent().len())
}

fn content_max_width(panel_width: u16, indent_cols: usize) -> usize {
    panel_width.saturating_sub(indent_cols as u16).max(1) as usize
}

fn format_system_help_body(body: &str) -> String {
    body.split(';')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(|part| format!("• {part}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn parse_system_session_row(body: &str) -> Option<(&str, &str, &str)> {
    let body = body.trim();
    let mut parts = body.split("  ").filter(|part| !part.is_empty());
    let id = parts.next()?;
    let date = parts.next()?;
    let title = parts.next()?;
    if !id.contains('-') || id.len() < 8 {
        return None;
    }
    Some((id, date, title))
}

fn system_session_row_line(th: ThemePalette, id: &str, date: &str, title: &str) -> Line<'static> {
    let short_id = coworker_core::agent::context::truncate_chars(id, 8);
    let indent = message_continuation_indent("system");
    Line::from(vec![
        Span::raw(indent),
        Span::styled(
            short_id,
            Style::default().fg(th.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  {date}  "),
            Style::default().fg(th.muted).add_modifier(Modifier::ITALIC),
        ),
        Span::styled(title.to_string(), Style::default().fg(th.text)),
    ])
}

fn meta_message_lines(
    th: ThemePalette,
    kind: &str,
    body: &str,
    max_width: Option<usize>,
) -> Vec<Line<'static>> {
    let body = body.trim();

    if kind == "system" {
        if let Some((id, date, title)) = parse_system_session_row(body) {
            return vec![system_session_row_line(th, id, date, title)];
        }
    }

    let rendered_body = if kind == "system" && body.contains('/') && body.contains(';') {
        format_system_help_body(body)
    } else {
        body.to_string()
    };
    let markdown = rendered_body.contains("• ");
    message_lines(
        th,
        kind,
        th.muted,
        th.title_bg,
        &rendered_body,
        markdown,
        max_width,
    )
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
        if coworker_core::agent::context::is_tool_result_transcript(rest) {
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
                Style::default().fg(th.warn).add_modifier(Modifier::ITALIC),
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
    if let Some(rest) = line.strip_prefix("system> ") {
        return meta_message_lines(th, "system", rest, max_width);
    }
    if let Some(rest) = line.strip_prefix("chat> ") {
        return meta_message_lines(th, "chat", rest, max_width);
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

fn tail_body_indent() -> String {
    "        ".to_string()
}
pub fn normalize_message_layout(body: &str) -> String {
    let body = coworker_core::terminal::sanitize_terminal_output(body);
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

/// Panel-background gap between the role badge and message body on the first line.
const MESSAGE_BODY_GAP: &str = " ";

fn message_lines(
    th: ThemePalette,
    role: &str,
    accent: Color,
    badge_bg: Color,
    body: &str,
    markdown: bool,
    max_width: Option<usize>,
) -> Vec<Line<'static>> {
    let content_style = Style::default().fg(match role {
        "You" => th.user,
        "system" | "chat" => th.text,
        _ => th.assistant,
    });
    let mut content_lines = if markdown {
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
    if let Some(mw) = max_width.filter(|w| *w > 0) {
        let panel = mw + message_indent_width();
        let mut wrapped = Vec::new();
        for (i, line) in content_lines.into_iter().enumerate() {
            let prefix_w = if i == 0 {
                first_line_prefix_width(role)
            } else {
                message_prefix_width(role)
            };
            let budget = panel.saturating_sub(prefix_w).max(1);
            wrapped.extend(super::markdown::wrap_content_lines(vec![line], budget));
        }
        content_lines = wrapped;
    }

    let indent = message_continuation_indent(role);
    let bar = Span::styled("▌ ", Style::default().fg(accent));
    let badge = role_badge(th, role, badge_bg);

    let mut out = Vec::with_capacity(content_lines.len().max(1));
    for (i, content) in content_lines.into_iter().enumerate() {
        if i == 0 {
            let mut spans = vec![bar.clone(), badge.clone(), Span::raw(MESSAGE_BODY_GAP)];
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
    message_prefix_width("You")
}

fn message_prefix_width(role: &str) -> usize {
    UnicodeWidthStr::width("▌ ")
        + UnicodeWidthStr::width(format!(" {role} ").as_str())
        + UnicodeWidthStr::width(MESSAGE_BODY_GAP)
}

fn first_line_prefix_width(role: &str) -> usize {
    message_prefix_width(role)
}

fn message_continuation_indent(role: &str) -> String {
    let cols = message_prefix_width(role).max(message_indent_width());
    " ".repeat(cols)
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
    let body = coworker_core::terminal::sanitize_terminal_output(body);
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
    let Some((name, body)) = coworker_core::agent::context::split_tool_transcript(transcript)
    else {
        return format_tool_detail_lines(th, transcript);
    };
    let mut out = vec![Line::from(vec![
        Span::raw("      "),
        Span::styled("✓ ", Style::default().fg(th.ok)),
        Span::styled(name, Style::default().fg(th.muted)),
        Span::styled(
            " (transcript)",
            Style::default().fg(th.muted).add_modifier(Modifier::ITALIC),
        ),
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
    fn bash_tool_transcript_strips_curl_carriage_returns() {
        let th = ThemePalette::dark();
        let curl_stderr = "  % Total    % Received % Xferd  Average Speed\r  0     0    0     0\r100  116k  100  116k    0     0   101k      0  0:00:01  0:00:01 --:--:--  101k\n";
        let body = format!(
            "tool_result(bash_run):\nbash_run: `curl -L https://example.com -o out.html`\nexit: 0 (1200ms)\n\nstderr:\n{curl_stderr}"
        );
        let rows = format_tool_transcript_lines(th, &body, Some(60));
        let joined: String = rows
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.as_ref())
            .collect();
        assert!(
            !joined.contains('\r'),
            "carriage returns must not reach TUI spans"
        );
        assert!(joined.contains("100  116k"));
        assert!(!joined.contains("% Total"));
    }

    #[test]
    fn assistant_tool_transcript_uses_tool_style_not_ai_badge() {
        let th = ThemePalette::dark();
        let raw =
            "assistant> tool_result(pr_list_changed_files, pr_number=19275):\n1 changed file(s)";
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
    fn system_and_user_messages_share_badge_layout() {
        let th = ThemePalette::dark();
        let system = format_chat_lines(th, "system> recent sessions:", None);
        let user = format_chat_lines(th, "you> hello", None);
        let sys_first: String = system[0].spans.iter().map(|s| s.content.as_ref()).collect();
        let you_first: String = user[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(
            sys_first.starts_with('▌'),
            "system should use solid bar: {sys_first:?}"
        );
        assert!(
            you_first.starts_with('▌'),
            "user should use solid bar: {you_first:?}"
        );
        assert!(
            sys_first.contains(" system "),
            "expected system badge: {sys_first:?}"
        );
        assert!(
            you_first.contains(" You "),
            "expected You badge: {you_first:?}"
        );
    }

    #[test]
    fn system_message_uses_role_layout() {
        let th = ThemePalette::dark();
        let rows = format_chat_lines(th, "system> exported to /tmp/chat.md", None);
        let joined: String = rows
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.as_ref())
            .collect();
        assert!(joined.contains('▌'));
        assert!(joined.contains("system"));
        assert!(joined.contains("exported"));
        assert!(!joined.starts_with("system>"));
    }

    #[test]
    fn system_help_splits_into_bullets() {
        let th = ThemePalette::dark();
        let rows = format_chat_lines(
            th,
            "system> /clear /new — transcript; /sessions — list; /export — save",
            None,
        );
        assert!(rows.len() >= 2, "expected bullet list, got {}", rows.len());
        let joined: String = rows
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.as_ref())
            .collect();
        assert!(joined.contains("/clear"));
        assert!(joined.contains("/sessions"));
    }

    #[test]
    fn system_session_row_formats_columns() {
        let th = ThemePalette::dark();
        let rows = format_chat_lines(
            th,
            "system> 1080bda6-820d-4d72-907d-0be336b127aa  06-12 14:30  CI triage for PR #42",
            None,
        );
        assert_eq!(rows.len(), 1);
        let joined: String = rows[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(joined.contains("1080bda6"));
        assert!(joined.contains("06-12"));
        assert!(joined.contains("CI triage"));
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
    fn none_palette_uses_terminal_defaults() {
        let th = ThemePalette::none();
        assert_eq!(th.bg, Color::Reset);
        assert_eq!(th.surface, Color::Reset);
        assert_eq!(th.accent, Color::Cyan);
        assert_ne!(th.bg, ThemePalette::dark().bg);
    }

    #[test]
    fn custom_accent_from_tui_config() {
        let tui = coworker_core::config::TuiConfig {
            accent: Some("#ff5500".into()),
            ..Default::default()
        };
        let th = ThemePalette::from_tui(&tui, ThemeMode::Dark);
        assert_eq!(th.accent, Color::Rgb(255, 85, 0));
        assert_eq!(th.accent_dim, Color::Rgb(170, 56, 0));
    }

    #[test]
    fn pr_glyphs_reflect_status() {
        assert_eq!(pr_ci_glyph("passing (3/3)"), "✓");
        assert_eq!(pr_ci_glyph("failing (1/3)"), "✗");
        assert_eq!(pr_review_glyph("review-required"), "◉");
        assert_eq!(pr_review_glyph("approved"), "✓");
    }

    #[test]
    fn assistant_message_fits_narrow_panel_with_badge() {
        let th = ThemePalette::dark();
        let panel = 30u16;
        let content_w = chat_content_max_width(panel);
        let body =
            "The GitHub secretary will analyze CI failures and suggest reruns for flaky jobs.";
        let rows = format_assistant_message_lines(th, body, Some(content_w));
        assert!(!rows.is_empty());
        for line in &rows {
            assert!(
                line.width() <= panel as usize,
                "line wider than panel: {:?}",
                line
            );
        }
        let joined: String = rows
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.as_ref())
            .collect();
        assert!(joined.contains("GitHub"));
        assert!(!joined.contains("isHu"));
    }

    #[test]
    fn message_badge_has_body_gap() {
        let th = ThemePalette::dark();
        let rows = format_chat_lines(th, "you> hello", Some(40));
        let first: String = rows[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(first.contains("You"), "expected badge: {first:?}");
        assert!(
            first.contains("You hello") || first.contains("You  hello"),
            "expected gap between badge and body: {first:?}"
        );
    }

    #[test]
    fn user_url_fits_split_pane_width() {
        let th = ThemePalette::dark();
        let panel = 50u16;
        let content_w = chat_content_max_width(panel);
        let url =
            "https://github.com/acme/widget/actions/runs/27400805815/job/12345678901?pr=19194";
        let rows = format_chat_lines(
            th,
            &format!("you> Read this PR runs: {url}"),
            Some(content_w),
        );
        let fitted = crate::markdown::ensure_chat_lines_fit_panel(rows, panel);
        assert!(
            fitted
                .iter()
                .all(|line| { crate::markdown::line_display_width(line) <= panel as usize }),
            "long URLs must wrap inside the Messages pane"
        );
    }

    #[test]
    fn tool_detail_lines_indent() {
        let th = ThemePalette::dark();
        let rows = format_tool_detail_lines(th, "line one\nline two");
        assert_eq!(rows.len(), 2);
        assert!(rows[0].spans[0].content.starts_with("        "));
    }

    #[test]
    fn tab_at_column_resolves_header_labels() {
        use coworker_core::app::Tab;
        let tabs = vec![Tab::Approvals, Tab::Logs, Tab::Config];
        assert_eq!(tab_at_column(&tabs, 0), Some(Tab::Approvals));
        let logs_start = UnicodeWidthStr::width(" 1 Approvals │");
        assert_eq!(tab_at_column(&tabs, logs_start), Some(Tab::Logs));
    }
}
