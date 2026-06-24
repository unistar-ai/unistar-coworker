use std::sync::Mutex;

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Scrollbar, ScrollbarOrientation};

use crate::agent::budget::TokenBudget;
use crate::agent::chat_loop::ContextSnapshot;
use crate::app::AppState;
use crate::tui::markdown::{self, markdown_to_lines_in_width};
use crate::tui::scroll::paragraph_scrollbar_state;
use crate::tui::theme::{self, ThemePalette};

const CONTEXT_SCROLL_PAGE: u16 = 8;

struct ContextRenderCache {
    revision: u64,
    width: u16,
    lines: Vec<Line<'static>>,
}

static RENDER_CACHE: Mutex<ContextRenderCache> = Mutex::new(ContextRenderCache {
    revision: 0,
    width: 0,
    lines: Vec::new(),
});

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

pub fn format_context_usage(used: u32, budget: u32) -> String {
    format!(
        "{} / {} ({:.1}%)",
        format_tokens(used),
        format_tokens(budget),
        context_usage_pct(used, budget)
    )
}

pub fn context_snapshot_usage(snap: &ContextSnapshot) -> (u32, u32) {
    (snap.total_tokens(), snap.input_budget)
}

pub fn context_status_spans(th: ThemePalette, state: &AppState) -> Vec<Span<'static>> {
    if state.tab != crate::app::Tab::Chat {
        return Vec::new();
    }
    let surface = Style::default().bg(th.surface);
    let input_budget =
        TokenBudget::from_config(state.config.llm.context_limit).input_budget();
    let (used, budget) = if let Some(snap) = &state.chat_context {
        context_snapshot_usage(snap)
    } else {
        (0, input_budget)
    };
    let pct = context_usage_pct(used, budget);
    let usage_fg = if pct >= 80.0 {
        th.warn
    } else if pct >= 60.0 {
        th.accent
    } else {
        th.muted
    };
    let usage = if state.chat_context.is_some() {
        format_context_usage(used, budget)
    } else {
        format!("— / {}", format_tokens(budget))
    };
    vec![
        Span::styled(" │ ctx ", Style::default().fg(th.border).bg(th.surface)),
        Span::styled(usage, Style::default().fg(usage_fg).bg(th.surface)),
        Span::styled(" ", surface),
    ]
}

/// Compact store snapshot on non-Chat tabs (digest / approvals / flaky / alerts).
pub fn store_status_spans(th: ThemePalette, state: &AppState) -> Vec<Span<'static>> {
    use crate::app::Tab;

    if state.tab == Tab::Chat {
        return Vec::new();
    }
    let surface = Style::default().bg(th.surface);
    let mut spans = vec![Span::styled(
        " │ store ",
        Style::default().fg(th.border).bg(th.surface),
    )];
    if let Some(d) = &state.latest_digest {
        let attn = d.summary.needs_attention;
        let attn_fg = if attn > 0 { th.warn } else { th.muted };
        spans.push(Span::styled(
            format!("digest {} ", d.date),
            Style::default().fg(th.muted).bg(th.surface),
        ));
        spans.push(Span::styled(
            format!("attn:{attn} "),
            Style::default().fg(attn_fg).bg(th.surface),
        ));
        if !d.summary.complete {
            spans.push(Span::styled(
                "◷ ",
                Style::default().fg(th.accent).bg(th.surface),
            ));
        }
    } else {
        spans.push(Span::styled(
            "no digest ",
            Style::default().fg(th.muted).bg(th.surface),
        ));
    }
    if !state.approvals.is_empty() {
        spans.push(Span::styled(
            format!("appr:{} ", state.approvals.len()),
            Style::default().fg(th.warn).bg(th.surface),
        ));
    }
    if state.attach_mode {
        spans.push(Span::styled(
            "attach ",
            Style::default().fg(th.accent).bg(th.surface),
        ));
    }
    spans.push(Span::styled(" ", surface));
    spans
}

pub fn token_bar(used: u32, limit: u32, width: usize) -> String {
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

const LATENCY_BAR_MAX_MS: u32 = 2000;

pub fn latency_bar_ms(ms: u128, width: usize) -> String {
    let used = ms.min(u128::from(LATENCY_BAR_MAX_MS)) as u32;
    token_bar(used, LATENCY_BAR_MAX_MS, width)
}

pub fn format_probe_latency(ok: bool, latency_ms: Option<u128>) -> String {
    if !ok {
        "offline".to_string()
    } else {
        latency_ms
            .map(|ms| format!("{ms}ms"))
            .unwrap_or_else(|| "ok".to_string())
    }
}

fn role_style(th: ThemePalette, display_role: &str) -> Style {
    let fg = match display_role {
        "system" => th.muted,
        "assistant" => th.assistant,
        "tool" => th.accent_dim,
        "harness" => th.warn,
        "reasoning" => th.accent,
        "skill" => th.accent,
        "tools" => th.accent,
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
        "skill" => th.text,
        "tools" => th.muted,
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
    content_max_width: usize,
) -> Vec<Line<'static>> {
    let content = crate::terminal::sanitize_terminal_output(content);
    let base = role_content_style(th, display_role);
    let mw = content_max_width.max(1);
    markdown_to_lines_in_width(th, &content, base, Some(mw))
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

fn push_context_section(
    lines: &mut Vec<Line<'static>>,
    th: ThemePalette,
    role: &str,
    label: &str,
    tokens: u32,
    body: &str,
    content_max_width: usize,
) {
    lines.push(Line::from(vec![
        Span::styled(format!("[{role}]"), role_style(th, role)),
        Span::styled(
            format!(" {label} · {}", format_message_tokens(tokens)),
            Style::default().fg(th.muted).add_modifier(Modifier::ITALIC),
        ),
    ]));
    lines.extend(render_message_content(
        th,
        role,
        body,
        content_max_width,
    ));
}

pub fn build_message_lines(
    th: ThemePalette,
    snapshot: Option<&ContextSnapshot>,
    content_max_width: usize,
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
    push_context_section(
        &mut lines,
        th,
        "tools",
        &format!("{} tool(s)", snap.tool_names.len()),
        snap.tools_tokens,
        &snap.tools_body,
        content_max_width,
    );
    lines.push(Line::from(""));

    for skill in &snap.skill_blocks {
        push_context_section(
            &mut lines,
            th,
            "skill",
            &skill.name,
            skill.tokens,
            &skill.body,
            content_max_width,
        );
        lines.push(Line::from(""));
    }

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
            content_max_width,
        ));
    }
    lines
}

fn context_viewport_lines(
    th: ThemePalette,
    state: &AppState,
    panel_width: u16,
    content_max_width: usize,
) -> Vec<Line<'static>> {
    let revision = state.chat_context_revision;
    let mut cache = RENDER_CACHE.lock().expect("context render cache");
    let stale = cache.revision != revision || cache.width != panel_width;
    if stale {
        let raw_lines = build_message_lines(th, state.chat_context.as_ref(), content_max_width);
        let pad_style = Style::default().bg(th.panel);
        cache.lines =
            markdown::finalize_panel_lines(raw_lines, panel_width, pad_style, false);
        cache.revision = revision;
        cache.width = panel_width;
    }
    cache.lines.clone()
}

fn context_text_width(panel_width: u16, scrollbar: bool) -> u16 {
    let mut width = panel_width.max(1);
    if scrollbar {
        width = width.saturating_sub(1).max(1);
    }
    width
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
        let line = Line::from(vec![Span::styled("  — / —", Style::default().fg(th.muted))]);
        frame.render_widget(Paragraph::new(line), inner);
        return;
    };

    let total = snap.total_tokens();
    let over = total > snap.input_budget;
    let near_cap = total > snap.input_budget.saturating_mul(85) / 100;
    let usage_style = Style::default().fg(if over {
        th.err
    } else if near_cap {
        th.warn
    } else {
        th.text
    });

    let bar_w = inner
        .width
        .saturating_sub(4)
        .max(8) as usize;
    let mut header = vec![
        Line::from(vec![
            Span::styled("input ", Style::default().fg(th.muted)),
            Span::styled(
                format_context_usage(total, snap.input_budget),
                usage_style.add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![Span::styled(
            format!("  {}", token_bar(total, snap.input_budget, bar_w)),
            Style::default().fg(if over { th.err } else { th.accent }),
        )]),
    ];
    let mut meta = vec![
        Span::styled("msgs ", Style::default().fg(th.muted)),
        Span::styled(format_tokens(snap.message_tokens), Style::default().fg(th.text)),
        Span::styled("  ·  tools ", Style::default().fg(th.muted)),
        Span::styled(
            format!(
                "{} ({})",
                format_tokens(snap.tools_tokens),
                snap.tool_names.len()
            ),
            Style::default().fg(th.text),
        ),
        Span::styled("  ·  skills ", Style::default().fg(th.muted)),
        Span::styled(
            format!(
                "{} ({})",
                format_tokens(snap.skills_tokens),
                snap.skill_blocks.len()
            ),
            Style::default().fg(th.text),
        ),
        Span::styled("  ·  ", Style::default().fg(th.muted)),
        Span::styled(
            format!("{} msgs", snap.message_count),
            Style::default().fg(th.muted),
        ),
        Span::styled("  ·  step ", Style::default().fg(th.muted)),
        Span::styled(format!("{}", snap.turn), Style::default().fg(th.muted)),
    ];
    if let Some(rev) = snap.runtime_context_revision {
        meta.push(Span::styled("  ·  ctx rev ", Style::default().fg(th.muted)));
        meta.push(Span::styled(format!("{rev}"), Style::default().fg(th.muted)));
    }
    meta.push(Span::styled("  ·  win ", Style::default().fg(th.muted)));
    meta.push(Span::styled(
        format_tokens(snap.context_limit),
        Style::default().fg(th.muted),
    ));
    header.push(Line::from(meta));
    let header_bg = Style::default().bg(th.panel);
    header = markdown::pad_lines_to_panel_width(
        markdown::reflow_chat_lines_to_width(header, inner.width),
        inner.width,
        header_bg,
    );
    if header.len() as u16 > inner.height {
        header.truncate(inner.height as usize);
    }
    frame.render_widget(Clear, inner);
    frame.render_widget(
        Paragraph::new(Text::from(header)).style(header_bg),
        inner,
    );
}

pub fn draw_context_panel(frame: &mut ratatui::Frame, area: Rect, state: &AppState) {
    let th = theme::ThemePalette::from_tui(&state.config.tui, state.config.theme());
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

    let panel_w = inner.width.max(1);
    let text_w = context_text_width(panel_w, true);
    let content_w = theme::context_content_max_width(text_w);
    let lines = context_viewport_lines(th, state, text_w, content_w);

    let visible = inner.height.max(1);
    let total = lines.len().min(u16::MAX as usize) as u16;
    let max_scroll = total.saturating_sub(visible);
    let scroll_from_bottom = state.chat_context_scroll_from_bottom.min(max_scroll);
    let scroll_y = max_scroll.saturating_sub(scroll_from_bottom);

    frame.render_widget(Clear, inner);
    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .style(Style::default().bg(th.panel))
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::chat_loop::{ContextLine, ContextSnapshot};
    use ratatui::style::Modifier;

    #[test]
    fn context_panel_shows_tools_section() {
        let th = theme::ThemePalette::dark();
        let snap = ContextSnapshot {
            turn: 1,
            message_tokens: 100,
            tools_tokens: 250,
            tools_body: "### tool_search\nSearch\n\n```json\n{}\n```".into(),
            tool_names: vec!["tool_search".into(), "pr_get_overview".into()],
            skill_blocks: vec![],
            skills_tokens: 0,
            input_budget: 52_523,
            context_limit: 64_000,
            message_count: 1,
            messages: vec![ContextLine {
                display_role: "user".into(),
                content: "hello".into(),
                tokens: 10,
            }],
            ..Default::default()
        };
        let lines = build_message_lines(th, Some(&snap), 72);
        let joined: String = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.as_ref())
            .collect();
        assert!(joined.contains("[tools]"));
        assert!(joined.contains("tool_search"));
        assert!(joined.contains("Search"));
    }

    #[test]
    fn context_panel_shows_skill_section() {
        use crate::agent::chat_loop::ContextSkillBlock;
        let th = theme::ThemePalette::dark();
        let snap = ContextSnapshot {
            turn: 1,
            message_tokens: 200,
            tools_tokens: 0,
            tools_body: "(none)".into(),
            tool_names: vec![],
            skill_blocks: vec![ContextSkillBlock {
                name: "ci-triage".into(),
                body: "## Rules\n- flaky vs real".into(),
                tokens: 40,
            }],
            skills_tokens: 40,
            input_budget: 52_523,
            context_limit: 64_000,
            message_count: 1,
            messages: vec![ContextLine {
                display_role: "system".into(),
                content: "Agent only".into(),
                tokens: 20,
            }],
            ..Default::default()
        };
        let lines = build_message_lines(th, Some(&snap), 72);
        let joined: String = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.as_ref())
            .collect();
        assert!(joined.contains("[skill]"));
        assert!(joined.contains("ci-triage"));
        assert!(joined.contains("flaky vs real"));
    }

    #[test]
    fn format_context_usage_shows_limit_and_pct() {
        assert_eq!(
            format_context_usage(12_400, 52_523),
            "12.4k / 52.5k (23.6%)"
        );
    }

    #[test]
    fn token_bar_fits_declared_width() {
        let panel = 38usize;
        let bar_w = panel.saturating_sub(4).max(8);
        let line = format!("  {}", token_bar(32_000, 64_000, bar_w));
        assert!(
            unicode_width::UnicodeWidthStr::width(line.as_str()) <= panel,
            "token bar row must fit panel: {line:?}"
        );
    }

    #[test]
    fn token_bar_fills_proportionally() {
        let bar = token_bar(32_000, 64_000, 10);
        assert!(bar.starts_with("[█████"));
    }

    #[test]
    fn latency_bar_scales_to_two_seconds() {
        let bar = latency_bar_ms(1000, 10);
        assert!(bar.starts_with("[█████"));
        assert_eq!(format_probe_latency(true, Some(42)), "42ms");
        assert_eq!(format_probe_latency(false, Some(42)), "offline");
    }

    #[test]
    fn store_status_spans_skip_chat_tab() {
        let th = theme::ThemePalette::dark();
        let yaml = r#"
llm: { base_url: http://localhost:11434/v1, model: m, context_limit: 64000 }
storage: { backend: json, path: ./data }
repos: [acme/widget]
"#;
        let cfg: crate::config::Config = serde_yaml::from_str(yaml).unwrap();
        let mut state = AppState::new(cfg, "coworker.yaml".into());
        state.tab = crate::app::Tab::Dashboard;
        let spans = store_status_spans(th, &state);
        let joined: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(joined.contains("store"));
        state.tab = crate::app::Tab::Chat;
        assert!(store_status_spans(th, &state).is_empty());
    }

    #[test]
    fn context_message_body_renders_markdown() {
        let th = theme::ThemePalette::dark();
        let snap = ContextSnapshot {
            turn: 2,
            message_tokens: 100,
            tools_tokens: 0,
            tools_body: String::new(),
            tool_names: vec![],
            skill_blocks: vec![],
            skills_tokens: 0,
            input_budget: 40_000,
            context_limit: 64_000,
            message_count: 1,
            messages: vec![ContextLine {
                display_role: "assistant".into(),
                content: "**PR #19264** — CI failing\n- check logs\n- retry".into(),
                tokens: 42,
            }],
            ..Default::default()
        };
        let lines = build_message_lines(
            th,
            Some(&snap),
            theme::context_content_max_width(72),
        );
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
        let th = theme::ThemePalette::dark();
        let snap = ContextSnapshot {
            turn: 1,
            message_tokens: 50,
            tools_tokens: 0,
            tools_body: String::new(),
            tool_names: vec![],
            skill_blocks: vec![],
            skills_tokens: 0,
            input_budget: 40_000,
            context_limit: 64_000,
            message_count: 1,
            messages: vec![ContextLine {
                display_role: "system".into(),
                content: "word ".repeat(60),
                tokens: 10,
            }],
            ..Default::default()
        };
        let width = 48u16;
        let content_w = theme::context_content_max_width(width);
        let raw = build_message_lines(th, Some(&snap), content_w);
        let lines = markdown::finalize_panel_lines(
            raw,
            width,
            Style::default(),
            false,
        );
        assert!(
            lines.len() > 1,
            "non-table context body should wrap in the panel"
        );
        assert!(
            lines.iter().all(|l| {
                markdown::line_display_width_for_test(l) == width as usize
            }),
            "each context row must be exactly panel width after padding"
        );
    }

    #[test]
    fn context_panel_wraps_long_markdown() {
        let th = theme::ThemePalette::dark();
        let snap = ContextSnapshot {
            turn: 1,
            message_tokens: 50,
            tools_tokens: 0,
            tools_body: String::new(),
            tool_names: vec![],
            skill_blocks: vec![],
            skills_tokens: 0,
            input_budget: 40_000,
            context_limit: 64_000,
            message_count: 1,
            messages: vec![ContextLine {
                display_role: "assistant".into(),
                content: format!("**Summary:** {}", "detail ".repeat(40)),
                tokens: 10,
            }],
            ..Default::default()
        };
        let width = 48u16;
        let content_w = theme::context_content_max_width(width);
        let raw = build_message_lines(th, Some(&snap), content_w);
        let lines = crate::tui::markdown::reflow_chat_lines_to_width(raw, width);
        assert!(
            lines.len() > 1,
            "markdown paragraphs should wrap in the panel"
        );
    }

    #[test]
    fn context_panel_table_rows_stay_single_line() {
        let th = theme::ThemePalette::dark();
        let snap = ContextSnapshot {
            turn: 1,
            message_tokens: 50,
            tools_tokens: 0,
            tools_body: String::new(),
            tool_names: vec![],
            skill_blocks: vec![],
            skills_tokens: 0,
            input_budget: 40_000,
            context_limit: 64_000,
            message_count: 1,
            messages: vec![ContextLine {
                display_role: "tool".into(),
                content: "| Tool | Notes |\n|------|-------|\n| pr_get_overview | snapshot |\n| pr_list_open | list |".into(),
                tokens: 10,
            }],
            ..Default::default()
        };
        let width = 36u16;
        let content_w = theme::context_content_max_width(width);
        let raw = build_message_lines(th, Some(&snap), content_w);
        let lines = crate::tui::markdown::reflow_chat_lines_to_width(raw, width);
        let table_line_count = lines
            .iter()
            .filter(|l| {
                let text: String = l.spans.iter().map(|s| s.content.as_ref()).collect();
                let t = text.trim();
                t.starts_with('│') || (t.starts_with('├') && t.ends_with('┤'))
            })
            .count();
        assert!(table_line_count >= 2, "expected formatted table rows");
        for line in &lines {
            let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            let t = text.trim();
            if t.starts_with('│') || (t.starts_with('├') && t.ends_with('┤')) {
                assert!(
                    unicode_width::UnicodeWidthStr::width(text.as_str()) <= width as usize,
                    "table row wider than panel: {text:?}"
                );
            }
        }
    }

    #[test]
    fn context_tool_log_fits_panel_width() {
        let th = theme::ThemePalette::dark();
        let snap = ContextSnapshot {
            turn: 1,
            message_tokens: 50,
            tools_tokens: 0,
            tools_body: String::new(),
            tool_names: vec![],
            skill_blocks: vec![],
            skills_tokens: 0,
            input_budget: 40_000,
            context_limit: 64_000,
            message_count: 1,
            messages: vec![ContextLine {
                display_role: "tool".into(),
                content: "tool_result(ci_get_failed_logs):\n##[error]Process completed with exit code 1.\nrun / unit tests / test\tUNKNOWN STEP\t        AssertionError: expected 1 to equal 2".into(),
                tokens: 10,
            }],
            ..Default::default()
        };
        let width = 30u16;
        let content_w = theme::context_content_max_width(width);
        let raw = build_message_lines(th, Some(&snap), content_w);
        let lines = crate::tui::markdown::reflow_chat_lines_to_width(raw, width);
        assert!(
            lines.iter().all(|l| {
                crate::tui::markdown::line_display_width_for_test(l) <= width as usize
            }),
            "CI log lines must fit narrow context panel"
        );
    }

    #[test]
    fn context_panel_shows_reasoning_summary_role() {
        let th = theme::ThemePalette::dark();
        let snap = ContextSnapshot {
            turn: 1,
            message_tokens: 120,
            tools_tokens: 0,
            tools_body: String::new(),
            tool_names: vec![],
            skill_blocks: vec![],
            skills_tokens: 0,
            input_budget: 40_000,
            context_limit: 64_000,
            message_count: 1,
            messages: vec![ContextLine {
                display_role: "reasoning".into(),
                content: "- checked CI on PR #42\n- will fetch diff".into(),
                tokens: 30,
            }],
            ..Default::default()
        };
        let lines = build_message_lines(
            th,
            Some(&snap),
            theme::context_content_max_width(72),
        );
        let joined = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.clone())
            .collect::<String>();
        assert!(
            joined.contains("[reasoning]"),
            "expected reasoning role label"
        );
        assert!(joined.contains("PR #42"), "expected summary body");
        assert!(
            !joined.contains("[agent reasoning summary]"),
            "marker should be stripped from context panel body"
        );
    }

    #[test]
    fn context_render_cache_reuses_lines_until_revision_changes() {
        let th = theme::ThemePalette::dark();
        let yaml = r#"
llm: { base_url: http://localhost:11434/v1, model: m, context_limit: 64000 }
storage: { backend: json, path: ./data }
repos: [acme/widget]
"#;
        let cfg: crate::config::Config = serde_yaml::from_str(yaml).unwrap();
        let mut state = AppState::new(cfg, "coworker.yaml".into());
        state.set_chat_context(ContextSnapshot {
            turn: 1,
            message_tokens: 50,
            tools_tokens: 0,
            tools_body: String::new(),
            tool_names: vec![],
            skill_blocks: vec![],
            skills_tokens: 0,
            input_budget: 40_000,
            context_limit: 64_000,
            message_count: 1,
            messages: vec![ContextLine {
                display_role: "user".into(),
                content: "hello".into(),
                tokens: 3,
            }],
            ..Default::default()
        });
        let w = 40u16;
        let cw = theme::context_content_max_width(w);
        let a = context_viewport_lines(th, &state, w, cw);
        let b = context_viewport_lines(th, &state, w, cw);
        assert_eq!(a, b);
        state.set_chat_context(ContextSnapshot {
            turn: 2,
            message_tokens: 60,
            tools_tokens: 0,
            tools_body: String::new(),
            tool_names: vec![],
            skill_blocks: vec![],
            skills_tokens: 0,
            input_budget: 40_000,
            context_limit: 64_000,
            message_count: 1,
            messages: vec![ContextLine {
                display_role: "user".into(),
                content: "updated".into(),
                tokens: 4,
            }],
            ..Default::default()
        });
        let c = context_viewport_lines(th, &state, w, cw);
        assert_ne!(a, c);
    }
}
