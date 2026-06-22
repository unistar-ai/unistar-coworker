use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use std::collections::HashSet;
use unicode_width::UnicodeWidthStr;

use crate::agent::context::truncate_chars;

use super::theme::ThemePalette;

/// ATX `##` section titles in document order (excludes `###` and deeper).
pub fn markdown_h2_section_titles(input: &str) -> Vec<String> {
    input
        .lines()
        .filter_map(|line| {
            let t = line.trim();
            if !t.starts_with("## ") || t.starts_with("### ") {
                return None;
            }
            Some(t.trim_start_matches("## ").trim().to_string())
        })
        .collect()
}

/// Hide body text under folded `##` sections; headers stay visible with a fold hint.
pub fn filter_folded_markdown_sections(
    input: &str,
    folded: &HashSet<String>,
    expand_hint: &str,
) -> String {
    if folded.is_empty() {
        return input.to_string();
    }
    let mut out = String::new();
    let mut in_folded = false;
    let mut skipped_lines = 0usize;
    for line in input.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("## ") && !trimmed.starts_with("### ") {
            if in_folded && skipped_lines > 0 {
                out.push_str(&format!(
                    "\n  _… {skipped_lines} line(s) folded — {expand_hint} to expand_"
                ));
            }
            skipped_lines = 0;
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(line);
            let title = trimmed.trim_start_matches("## ").trim();
            in_folded = folded.contains(title);
            if in_folded {
                out.push_str(" `[folded]`");
            }
            continue;
        }
        if in_folded {
            if !trimmed.is_empty() {
                skipped_lines += 1;
            }
            continue;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(line);
    }
    if in_folded && skipped_lines > 0 {
        out.push_str(&format!(
            "\n  _… {skipped_lines} line(s) folded — {expand_hint} to expand_"
        ));
    }
    out
}

/// Incremental markdown renderer for streaming assistant output.
///
/// Complete lines (ending in `\n`) are cached; only the trailing partial line is re-parsed.
pub struct StreamingMarkdownRenderer {
    source: String,
    stable_byte_len: usize,
    stable_lines: Vec<Line<'static>>,
}

impl StreamingMarkdownRenderer {
    pub fn new() -> Self {
        Self {
            source: String::new(),
            stable_byte_len: 0,
            stable_lines: Vec::new(),
        }
    }

    pub fn clear(&mut self) {
        self.source.clear();
        self.stable_byte_len = 0;
        self.stable_lines.clear();
    }

    /// Render `input`, reusing cached lines for every complete line prefix.
    pub fn render(
        &mut self,
        th: ThemePalette,
        input: &str,
        base: Style,
        max_width: Option<usize>,
    ) -> Vec<Line<'static>> {
        if input.is_empty() {
            self.clear();
            return Vec::new();
        }

        if input.len() < self.source.len() || !input.starts_with(self.source.as_str()) {
            self.clear();
        }

        if input != self.source {
            self.source = input.to_string();
            self.refresh_stable(th, base, max_width);
        } else if self.stable_lines.is_empty() {
            self.refresh_stable(th, base, max_width);
        }

        let tail = &self.source[self.stable_byte_len..];
        if tail.is_empty() {
            return self.stable_lines.clone();
        }
        let mut lines = self.stable_lines.clone();
        lines.extend(markdown_to_lines_in_width(th, tail, base, max_width));
        lines
    }

    fn refresh_stable(&mut self, th: ThemePalette, base: Style, max_width: Option<usize>) {
        let new_stable = stable_line_prefix_byte_len(&self.source);
        if new_stable == self.stable_byte_len {
            return;
        }
        self.stable_byte_len = new_stable;
        if new_stable == 0 {
            self.stable_lines.clear();
            return;
        }
        let prefix = &self.source[..new_stable];
        self.stable_lines = markdown_to_lines_in_width(th, prefix, base, max_width);
    }

    #[cfg(test)]
    fn stable_byte_len(&self) -> usize {
        self.stable_byte_len
    }
}

impl Default for StreamingMarkdownRenderer {
    fn default() -> Self {
        Self::new()
    }
}

/// Byte index after the last complete `\n` line in `s` (0 if none).
fn stable_line_prefix_byte_len(s: &str) -> usize {
    match s.rfind('\n') {
        Some(i) => i + 1,
        None => 0,
    }
}

/// Render a markdown fragment into styled terminal lines.
pub fn markdown_to_lines_in_width(
    th: ThemePalette,
    input: &str,
    base: Style,
    max_width: Option<usize>,
) -> Vec<Line<'static>> {
    let input = input.replace("\r\n", "\n");
    let input = promote_section_lines(&input);
    if should_preserve_line_breaks(&input) || !looks_like_markdown(&input) {
        let lines = plain_lines(th, &input, base);
        if let Some(mw) = max_width.filter(|w| *w > 0) {
            return wrap_content_lines(lines, mw);
        }
        return lines;
    }

    let mut renderer = MarkdownRenderer::new(th, base, max_width);
    let opts = Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES;
    for event in Parser::new_ext(&input, opts) {
        renderer.on_event(event);
    }
    wrap_rendered_lines(renderer.finish(), max_width)
}

fn plain_lines(th: ThemePalette, input: &str, base: Style) -> Vec<Line<'static>> {
    input
        .split('\n')
        .map(|line| {
            if line.is_empty() {
                Line::from("")
            } else {
                let line = normalize_terminal_text(line);
                enrich_pr_refs(th, Line::from(Span::styled(line, base)))
            }
        })
        .collect()
}

fn looks_like_markdown(s: &str) -> bool {
    s.contains("**")
        || s.contains("__")
        || s.contains('`')
        || s.contains("](")
        || s.lines().any(looks_like_table_row)
        || s.lines().any(looks_like_markdown_line)
}

/// Tool / MCP output and PR list rows must keep single `\n` breaks (not markdown-soft-break).
fn should_preserve_line_breaks(s: &str) -> bool {
    let trimmed = s.trim_start();
    if trimmed.starts_with("tool_result(")
        || trimmed.starts_with("tool_error(")
        || trimmed.starts_with("[tool_result ")
        || trimmed.starts_with("[summarized tool_result ")
    {
        return true;
    }
    let non_empty: Vec<&str> = s.lines().filter(|l| !l.trim().is_empty()).collect();
    if non_empty.len() < 2 {
        return false;
    }
    let pr_lines = non_empty
        .iter()
        .filter(|l| looks_like_pr_list_line(l))
        .count();
    pr_lines >= 2 && pr_lines * 2 >= non_empty.len()
}

fn looks_like_pr_list_line(line: &str) -> bool {
    let t = line.trim();
    if t.starts_with("open PR(s)") {
        return true;
    }
    if !t.starts_with('#') {
        return false;
    }
    t.chars().nth(1).is_some_and(|c| c.is_ascii_digit())
}

fn looks_like_markdown_line(line: &str) -> bool {
    let t = line.trim_start();
    looks_like_atx_heading(line)
        || t.starts_with("- ")
        || t.starts_with("* ")
        || t.starts_with("+ ")
        || t.starts_with("> ")
        || (t.chars().next().is_some_and(|c| c.is_ascii_digit()) && t.contains(". "))
}

/// CommonMark ATX heading: `# title` (space after hashes). Not `#19264` PR refs.
fn looks_like_atx_heading(line: &str) -> bool {
    let t = line.trim();
    if !t.starts_with('#') {
        return false;
    }
    let n = t.chars().take_while(|c| *c == '#').count();
    if n == 0 || n > 6 {
        return false;
    }
    t.len() == n || t.as_bytes().get(n) == Some(&b' ')
}

fn looks_like_table_row(line: &str) -> bool {
    let t = line.trim();
    t.starts_with('|') && t.ends_with('|') && t.matches('|').count() >= 2
}

/// LLMs often emit `1. Section` / `A. Subsection` / `**Title**` instead of ATX `#` headings.
fn promote_section_lines(input: &str) -> String {
    let lines: Vec<&str> = input.lines().collect();
    let skip = numbered_list_lines_to_preserve(&lines);
    lines
        .iter()
        .enumerate()
        .map(|(idx, line)| {
            if skip[idx] {
                (*line).to_string()
            } else {
                promote_section_line(line)
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Consecutive `N. item` lines are an ordered list, not section headings.
fn numbered_list_lines_to_preserve(lines: &[&str]) -> Vec<bool> {
    let mut skip = vec![false; lines.len()];
    let mut i = 0;
    while i < lines.len() {
        while i < lines.len() && lines[i].trim().is_empty() {
            i += 1;
        }
        if i >= lines.len() || !looks_like_numbered_list_item(lines[i]) {
            i += 1;
            continue;
        }
        let start = i;
        let mut items = 0usize;
        while i < lines.len() {
            if lines[i].trim().is_empty() {
                i += 1;
                continue;
            }
            if looks_like_numbered_list_item(lines[i]) {
                items += 1;
                i += 1;
            } else {
                break;
            }
        }
        if items >= 2 {
            for j in start..i {
                if looks_like_numbered_list_item(lines[j]) {
                    skip[j] = true;
                }
            }
        }
    }
    skip
}

fn looks_like_numbered_list_item(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return false;
    }
    let bytes = trimmed.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    i > 0 && bytes.get(i) == Some(&b'.') && bytes.get(i + 1) == Some(&b' ')
}

fn promote_section_line(line: &str) -> String {
    if looks_like_atx_heading(line) {
        return line.to_string();
    }
    let trimmed = line.trim();
    if trimmed.is_empty() || line.starts_with(' ') || line.starts_with('\t') {
        return line.to_string();
    }
    if let Some(title) = standalone_bold_line(trimmed) {
        return format!("## {title}");
    }
    if looks_like_numbered_section(trimmed) {
        return format!("## {trimmed}");
    }
    if looks_like_lettered_section(trimmed) {
        return format!("### {trimmed}");
    }
    if looks_like_emoji_section(trimmed) {
        return format!("### {trimmed}");
    }
    line.to_string()
}

fn standalone_bold_line(s: &str) -> Option<String> {
    let inner = s.strip_prefix("**")?.strip_suffix("**")?;
    if inner.is_empty() || inner.contains("**") {
        return None;
    }
    Some(inner.to_string())
}

fn looks_like_numbered_section(s: &str) -> bool {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i == 0 || bytes.get(i) != Some(&b'.') || bytes.get(i + 1) != Some(&b' ') {
        return false;
    }
    let rest = &s[i + 2..];
    !rest.is_empty() && rest.chars().count() <= 120
}

fn looks_like_lettered_section(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_uppercase() {
        return false;
    }
    if chars.next() != Some('.') || chars.next() != Some(' ') {
        return false;
    }
    chars.next().is_some()
}

fn looks_like_emoji_section(s: &str) -> bool {
    const PREFIXES: &[&str] = &["✅", "⚠️", "❗", "❌", "🔴", "🟡", "🟢"];
    PREFIXES.iter().any(|p| s.starts_with(p))
        || (s.starts_with('!') && s.len() > 1 && s.as_bytes().get(1) == Some(&b' '))
}

fn heading_style(th: ThemePalette, level: HeadingLevel) -> Style {
    match level {
        HeadingLevel::H1 => Style::default()
            .fg(th.heading_h1)
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        HeadingLevel::H2 => Style::default()
            .fg(th.heading_h2)
            .add_modifier(Modifier::BOLD),
        HeadingLevel::H3 => Style::default()
            .fg(th.heading_h3)
            .add_modifier(Modifier::BOLD),
        HeadingLevel::H4 => Style::default()
            .fg(th.heading_h4)
            .add_modifier(Modifier::BOLD),
        HeadingLevel::H5 => Style::default()
            .fg(th.muted)
            .add_modifier(Modifier::BOLD | Modifier::ITALIC),
        HeadingLevel::H6 => Style::default().fg(th.muted).add_modifier(Modifier::ITALIC),
    }
}

struct TableBuilder {
    rows: Vec<Vec<String>>,
    current_row: Vec<String>,
    current_cell: String,
    in_cell: bool,
}

impl TableBuilder {
    fn new() -> Self {
        Self {
            rows: Vec::new(),
            current_row: Vec::new(),
            current_cell: String::new(),
            in_cell: false,
        }
    }
}

fn is_table_separator_row(row: &[String]) -> bool {
    row.iter().all(|cell| {
        let t = cell.trim();
        !t.is_empty() && t.chars().all(|ch| ch == '-' || ch == ':' || ch == ' ')
    })
}

fn table_row_display_width(widths: &[usize], ncols: usize) -> usize {
    let sum: usize = widths.iter().take(ncols).sum();
    ncols + 1 + 2 * ncols + sum
}

fn fit_table_widths(widths: &mut [usize], ncols: usize, max_width: usize) {
    if max_width == 0 {
        return;
    }
    for _ in 0..4096 {
        if table_row_display_width(widths, ncols) <= max_width {
            return;
        }
        let Some((idx, _)) = widths
            .iter()
            .enumerate()
            .take(ncols)
            .max_by_key(|(_, w)| *w)
        else {
            break;
        };
        if widths[idx] <= 1 {
            break;
        }
        widths[idx] -= 1;
    }
}

fn normalize_terminal_text(s: &str) -> String {
    crate::terminal::sanitize_terminal_output(s).replace('\t', "    ")
}

/// Strip OSC/CSI sequences so display-width math matches ratatui layout (avoids double-wrap).
fn strip_terminal_escapes(s: &str) -> String {
    crate::terminal::strip_terminal_escapes(s)
}

fn visible_display_width(s: &str) -> usize {
    UnicodeWidthStr::width(strip_terminal_escapes(&normalize_terminal_text(s)).as_str())
}

fn truncate_to_display_width(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let s = strip_terminal_escapes(&normalize_terminal_text(s));
    if UnicodeWidthStr::width(s.as_str()) <= max {
        return s;
    }
    let mut out = String::new();
    let mut used = 0;
    for ch in s.chars() {
        let cw = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(1).max(1);
        if used + cw > max.saturating_sub(1) {
            out.push('…');
            return out;
        }
        used += cw;
        out.push(ch);
    }
    debug_assert!(UnicodeWidthStr::width(out.as_str()) <= max);
    out
}

#[derive(Clone, Copy)]
struct StyledChar {
    ch: char,
    style: Style,
}

pub(crate) fn line_display_width(line: &Line<'_>) -> usize {
    line.spans
        .iter()
        .map(|s| visible_display_width(s.content.as_ref()))
        .sum()
}

#[cfg(test)]
pub(crate) fn line_display_width_for_test(line: &Line<'_>) -> usize {
    line_display_width(line)
}

fn flatten_line(line: &Line<'_>) -> Vec<StyledChar> {
    let mut out = Vec::new();
    for span in &line.spans {
        for ch in span.content.chars() {
            out.push(StyledChar {
                ch,
                style: span.style,
            });
        }
    }
    out
}

fn chars_to_line(chars: Vec<StyledChar>) -> Line<'static> {
    if chars.is_empty() {
        return Line::from("");
    }
    let mut spans = Vec::new();
    let mut current = String::new();
    let mut current_style = chars[0].style;
    for sc in chars {
        if sc.style != current_style && !current.is_empty() {
            spans.push(Span::styled(std::mem::take(&mut current), current_style));
            current_style = sc.style;
        }
        current.push(sc.ch);
    }
    if !current.is_empty() {
        spans.push(Span::styled(current, current_style));
    }
    Line::from(spans)
}

fn styled_width(chars: &[StyledChar]) -> usize {
    chars
        .iter()
        .map(|sc| unicode_width::UnicodeWidthChar::width(sc.ch).unwrap_or(0))
        .sum()
}

fn split_into_words(chars: Vec<StyledChar>) -> Vec<Vec<StyledChar>> {
    let mut words = Vec::new();
    let mut current = Vec::new();
    let mut is_space = false;
    for sc in chars {
        if sc.ch.is_whitespace() {
            if !is_space && !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
            is_space = true;
            current.push(sc);
        } else {
            if is_space && !current.is_empty() {
                words.push(std::mem::take(&mut current));
                is_space = false;
            }
            current.push(sc);
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

/// Display width of list marker prefix only (`  1. ` or `    ▸ `), not the item body.
fn list_marker_prefix_display_width(text: &str) -> Option<usize> {
    if let Some(idx) = text.find('▸') {
        let byte_end = idx + '▸'.len_utf8();
        let end = if text.get(byte_end..).is_some_and(|s| s.starts_with(' ')) {
            byte_end + 1
        } else {
            byte_end
        };
        return Some(UnicodeWidthStr::width(&text[..end]));
    }
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i] == b' ' {
        i += 1;
    }
    let digit_start = i;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i > digit_start && bytes.get(i) == Some(&b'.') && bytes.get(i + 1) == Some(&b' ') {
        return Some(UnicodeWidthStr::width(&text[..i + 2]));
    }
    None
}

fn list_marker_hang_width(line: &Line<'_>) -> Option<usize> {
    let first = line.spans.first()?;
    list_marker_prefix_display_width(first.content.as_ref())
}

fn wrap_rendered_lines(lines: Vec<Line<'static>>, max_width: Option<usize>) -> Vec<Line<'static>> {
    let Some(width) = max_width.filter(|w| *w > 0) else {
        return lines;
    };
    lines
        .into_iter()
        .flat_map(|line| {
            if line_display_width(&line) <= width {
                vec![line]
            } else {
                let hang = list_marker_hang_width(&line);
                wrap_line_to_width(line, width, hang)
            }
        })
        .collect()
}

/// Word-wrap content rows before attaching message badges / indents.
pub(crate) fn wrap_content_lines(
    lines: Vec<Line<'static>>,
    max_width: usize,
) -> Vec<Line<'static>> {
    if max_width == 0 {
        return lines;
    }
    lines
        .into_iter()
        .flat_map(|line| {
            let hang = list_marker_hang_width(&line);
            if line_display_width(&line) <= max_width {
                vec![line]
            } else {
                wrap_line_to_width(line, max_width, hang)
            }
        })
        .collect()
}

fn truncate_line_to_width(line: Line<'static>, max_width: usize) -> Line<'static> {
    let text: String = line
        .spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect();
    let style = line
        .spans
        .first()
        .map(|s| s.style)
        .unwrap_or_default();
    Line::from(Span::styled(
        truncate_to_display_width(&text, max_width),
        style,
    ))
}

fn split_message_badge_line(line: &Line<'_>) -> Option<(Vec<Span<'static>>, Line<'static>)> {
    let first = line.spans.first()?;
    if !first.content.as_ref().starts_with('▌') || line.spans.len() < 2 {
        return None;
    }
    let mut prefix_end = 2usize;
    if line.spans.len() > 2 && line.spans[2].content.as_ref() == " " {
        prefix_end = 3;
    }
    let prefix: Vec<Span<'static>> = line.spans[..prefix_end]
        .iter()
        .map(|s| Span::styled(s.content.to_string(), s.style))
        .collect();
    let body: Line<'static> = Line::from(
        line.spans[prefix_end..]
            .iter()
            .map(|s| Span::styled(s.content.to_string(), s.style))
            .collect::<Vec<_>>(),
    );
    Some((prefix, body))
}

fn fit_chat_line_to_panel(line: Line<'static>, width: usize) -> Vec<Line<'static>> {
    if line_display_width(&line) <= width {
        return vec![line];
    }
    let hang = list_marker_hang_width(&line);
    if let Some((prefix, body)) = split_message_badge_line(&line) {
        let prefix_w: usize = prefix
            .iter()
            .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
            .sum();
        let body_budget = width.saturating_sub(prefix_w).max(1);
        let indent = " ".repeat(prefix_w);
        let wrapped = wrap_content_lines(vec![body], body_budget);
        return wrapped
            .into_iter()
            .enumerate()
            .map(|(i, wl)| {
                if i == 0 {
                    let mut spans = prefix.clone();
                    spans.extend(wl.spans);
                    Line::from(spans)
                } else {
                    let mut spans = vec![Span::raw(indent.clone())];
                    spans.extend(wl.spans);
                    Line::from(spans)
                }
            })
            .map(|row| {
                if line_display_width(&row) <= width {
                    row
                } else {
                    truncate_line_to_width(row, width)
                }
            })
            .collect();
    }
    let wrapped = wrap_line_to_width(line, width, hang);
    wrapped
        .into_iter()
        .map(|row| {
            if line_display_width(&row) <= width {
                row
            } else {
                truncate_line_to_width(row, width)
            }
        })
        .collect()
}

/// Pad or truncate each row to exactly `panel_width` columns so ratatui does not
/// leave ghost characters from a previous frame when a line gets shorter.
pub(crate) fn pad_lines_to_panel_width(
    lines: Vec<Line<'static>>,
    panel_width: u16,
    pad_style: Style,
) -> Vec<Line<'static>> {
    let width = panel_width.max(1) as usize;
    lines
        .into_iter()
        .map(|line| fit_line_to_panel_width(line, width, pad_style))
        .collect()
}

/// Reflow (optionally preserving chat badges), then pad every row to the pane width.
pub(crate) fn finalize_panel_lines(
    lines: Vec<Line<'static>>,
    panel_width: u16,
    pad_style: Style,
    preserve_message_badges: bool,
) -> Vec<Line<'static>> {
    let fitted = if preserve_message_badges {
        ensure_chat_lines_fit_panel(lines, panel_width)
    } else {
        reflow_chat_lines_to_width(lines, panel_width)
    };
    pad_lines_to_panel_width(fitted, panel_width, pad_style)
}

fn fit_line_to_panel_width(line: Line<'static>, width: usize, pad_style: Style) -> Line<'static> {
    let line = if line_display_width(&line) > width {
        truncate_line_to_width(line, width)
    } else {
        line
    };
    let used = line_display_width(&line);
    if used < width {
        let mut spans = line.spans;
        spans.push(Span::styled(" ".repeat(width - used), pad_style));
        Line::from(spans)
    } else {
        line
    }
}

/// Fit chat history rows to the Messages pane without splitting `▌ You` / `▌ AI` badges.
pub(crate) fn ensure_chat_lines_fit_panel(
    lines: Vec<Line<'static>>,
    panel_width: u16,
) -> Vec<Line<'static>> {
    let width = panel_width.max(1) as usize;
    lines
        .into_iter()
        .flat_map(|line| fit_chat_line_to_panel(line, width))
        .collect()
}

/// Force-wrap every row to `panel_width` (tables included). Used by the chat
/// pane where preserving wide table rows causes horizontal bleed in split view.
pub(crate) fn reflow_chat_lines_to_width(
    lines: Vec<Line<'static>>,
    panel_width: u16,
) -> Vec<Line<'static>> {
    let width = panel_width.max(1) as usize;
    let expanded: Vec<Line<'static>> = lines.into_iter().flat_map(expand_hard_newlines).collect();
    expanded
        .into_iter()
        .flat_map(|line| {
            let hang = list_marker_hang_width(&line);
            let wrapped = if line_display_width(&line) <= width {
                vec![line]
            } else {
                wrap_line_to_width(line, width, hang)
            };
            wrapped.into_iter().map(move |row| {
                if line_display_width(&row) <= width {
                    row
                } else {
                    truncate_line_to_width(row, width)
                }
            })
        })
        .collect()
}

fn expand_hard_newlines(line: Line<'static>) -> Vec<Line<'static>> {
    if !line.spans.iter().any(|span| span.content.contains('\n')) {
        return vec![line];
    }

    let mut rows: Vec<Vec<Span<'static>>> = vec![vec![]];
    for span in line.spans {
        let mut first = true;
        for part in span.content.split('\n') {
            if !first {
                rows.push(vec![]);
            }
            first = false;
            if part.is_empty() {
                continue;
            }
            rows.last_mut()
                .expect("row vec")
                .push(Span::styled(part.to_string(), span.style));
        }
    }
    rows.into_iter()
        .filter(|spans| !spans.is_empty())
        .map(Line::from)
        .collect()
}

fn wrap_line_to_width(
    line: Line<'static>,
    max_width: usize,
    hang_width: Option<usize>,
) -> Vec<Line<'static>> {
    let hang = hang_width.unwrap_or(0);
    let words = split_into_words(flatten_line(&line));
    if words.is_empty() {
        return vec![line];
    }

    let pad_style = words
        .iter()
        .find_map(|w| w.first().map(|sc| sc.style))
        .unwrap_or_default();

    let mut lines_out: Vec<Vec<StyledChar>> = vec![vec![]];
    let mut col = 0usize;
    let mut budget = max_width;

    let start_new_line =
        |lines_out: &mut Vec<Vec<StyledChar>>, col: &mut usize, budget: &mut usize| {
            lines_out.push(Vec::new());
            *col = 0;
            *budget = max_width;
            if hang > 0 {
                for _ in 0..hang {
                    lines_out.last_mut().expect("line vec").push(StyledChar {
                        ch: ' ',
                        style: pad_style,
                    });
                }
                *col = hang;
            }
        };

    for word in words {
        let ww = styled_width(&word);
        if ww == 0 {
            continue;
        }
        if col > 0 && col + ww > budget {
            start_new_line(&mut lines_out, &mut col, &mut budget);
        }
        if ww > budget && col == hang {
            for sc in word {
                let cw = unicode_width::UnicodeWidthChar::width(sc.ch).unwrap_or(0);
                if col + cw > budget {
                    if col > hang {
                        start_new_line(&mut lines_out, &mut col, &mut budget);
                    }
                    if col + cw > budget {
                        break;
                    }
                }
                lines_out.last_mut().expect("line vec").push(sc);
                col += cw;
            }
            continue;
        }
        lines_out.last_mut().expect("line vec").extend(word);
        col += ww;
    }

    lines_out.into_iter().map(chars_to_line).collect()
}

fn format_table_lines(
    th: ThemePalette,
    rows: Vec<Vec<String>>,
    base: Style,
    max_width: Option<usize>,
) -> Vec<Line<'static>> {
    if rows.is_empty() {
        return Vec::new();
    }
    let ncols = rows.iter().map(|r| r.len()).max().unwrap_or(0).max(1);
    let mut widths = vec![1usize; ncols];
    for row in &rows {
        for (i, cell) in row.iter().enumerate() {
            let w = UnicodeWidthStr::width(cell.trim());
            widths[i] = widths[i].max(w.max(1));
        }
    }
    if let Some(mw) = max_width {
        fit_table_widths(&mut widths, ncols, mw);
    }

    let mut lines = Vec::new();
    for (ri, row) in rows.iter().enumerate() {
        let mut spans = Vec::new();
        spans.push(Span::styled("│", Style::default().fg(th.muted)));
        for (i, width) in widths.iter().enumerate().take(ncols) {
            let raw = row.get(i).map(|s| s.trim()).unwrap_or("");
            let cell = truncate_to_display_width(raw, *width);
            let pad = width.saturating_sub(UnicodeWidthStr::width(cell.as_str()));
            spans.push(Span::styled(format!(" {cell}{} ", " ".repeat(pad)), base));
            spans.push(Span::styled("│", Style::default().fg(th.muted)));
        }
        lines.push(Line::from(spans));
        if ri == 0 && rows.len() > 1 {
            let sep = widths
                .iter()
                .map(|w| "─".repeat(w + 2))
                .collect::<Vec<_>>()
                .join("┼");
            lines.push(Line::from(Span::styled(
                format!("├{sep}┤"),
                Style::default().fg(th.muted),
            )));
        }
    }
    lines
}

fn json_code_line(th: ThemePalette, line: &str, base: Style) -> Line<'static> {
    let mut spans = vec![Span::raw("  │ ")];
    let bytes = line.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        if b.is_ascii_whitespace() {
            let start = i;
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            spans.push(Span::styled(line[start..i].to_string(), base));
            continue;
        }
        if b == b'"' {
            let start = i;
            i += 1;
            while i < bytes.len() {
                if bytes[i] == b'"' && bytes[i - 1] != b'\\' {
                    i += 1;
                    break;
                }
                i += 1;
            }
            let slice = &line[start..i];
            let rest = line[i..].trim_start();
            let style = if rest.starts_with(':') {
                Style::default().fg(th.accent)
            } else {
                Style::default().fg(th.ok)
            };
            spans.push(Span::styled(slice.to_string(), style));
            continue;
        }
        if bytes[i].is_ascii_digit() || bytes[i] == b'-' {
            let start = i;
            i += 1;
            while i < bytes.len()
                && (bytes[i].is_ascii_digit()
                    || matches!(bytes[i], b'.' | b'e' | b'E' | b'+' | b'-'))
            {
                i += 1;
            }
            spans.push(Span::styled(
                line[start..i].to_string(),
                Style::default().fg(th.warn),
            ));
            continue;
        }
        if line[i..].starts_with("false") {
            spans.push(Span::styled(
                line[i..i + 5].to_string(),
                Style::default().fg(th.muted),
            ));
            i += 5;
            continue;
        }
        if line[i..].starts_with("true") {
            spans.push(Span::styled(
                line[i..i + 4].to_string(),
                Style::default().fg(th.muted),
            ));
            i += 4;
            continue;
        }
        if line[i..].starts_with("null") {
            spans.push(Span::styled(
                line[i..i + 4].to_string(),
                Style::default().fg(th.muted),
            ));
            i += 4;
            continue;
        }
        spans.push(Span::styled(
            line[i..i + 1].to_string(),
            Style::default().fg(th.muted),
        ));
        i += 1;
    }
    Line::from(spans)
}

fn yaml_code_line(th: ThemePalette, line: &str, base: Style) -> Line<'static> {
    let trimmed = line.trim_start();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return Line::from(vec![
            Span::raw("  │ "),
            Span::styled(line.to_string(), Style::default().fg(th.muted)),
        ]);
    }
    if let Some((key, rest)) = trimmed.split_once(':') {
        let indent = line.len().saturating_sub(trimmed.len());
        let prefix = &line[..indent];
        return Line::from(vec![
            Span::raw("  │ "),
            Span::raw(prefix.to_string()),
            Span::styled(key.to_string(), Style::default().fg(th.accent)),
            Span::styled(":".to_string(), base),
            Span::styled(rest.to_string(), Style::default().fg(th.ok)),
        ]);
    }
    Line::from(vec![
        Span::raw("  │ "),
        Span::styled(line.to_string(), base),
    ])
}

fn rust_code_line(th: ThemePalette, line: &str, base: Style) -> Line<'static> {
    let trimmed = line.trim_start();
    if trimmed.starts_with("//") {
        return Line::from(vec![
            Span::raw("  │ "),
            Span::styled(
                line.to_string(),
                Style::default().fg(th.muted).add_modifier(Modifier::ITALIC),
            ),
        ]);
    }
    for kw in ["fn ", "func ", "type ", "struct ", "impl ", "use ", "mod ", "pub "] {
        if let Some(rest) = trimmed.strip_prefix(kw) {
            let indent = line.len().saturating_sub(trimmed.len());
            return Line::from(vec![
                Span::raw("  │ "),
                Span::raw(line[..indent].to_string()),
                Span::styled(
                    kw.to_string(),
                    Style::default().fg(th.accent).add_modifier(Modifier::BOLD),
                ),
                Span::styled(rest.to_string(), base),
            ]);
        }
    }
    Line::from(vec![
        Span::raw("  │ "),
        Span::styled(line.to_string(), base),
    ])
}

fn toml_code_line(th: ThemePalette, line: &str, base: Style) -> Line<'static> {
    yaml_code_line(th, line, base)
}

fn go_code_line(th: ThemePalette, line: &str, base: Style) -> Line<'static> {
    let trimmed = line.trim_start();
    if trimmed.starts_with("//") {
        return Line::from(vec![
            Span::raw("  │ "),
            Span::styled(
                line.to_string(),
                Style::default().fg(th.muted).add_modifier(Modifier::ITALIC),
            ),
        ]);
    }
    for kw in ["package ", "import ", "func ", "type ", "var ", "const "] {
        if let Some(rest) = trimmed.strip_prefix(kw) {
            let indent = line.len().saturating_sub(trimmed.len());
            return Line::from(vec![
                Span::raw("  │ "),
                Span::raw(line[..indent].to_string()),
                Span::styled(
                    kw.to_string(),
                    Style::default().fg(th.accent).add_modifier(Modifier::BOLD),
                ),
                Span::styled(rest.to_string(), base),
            ]);
        }
    }
    Line::from(vec![
        Span::raw("  │ "),
        Span::styled(line.to_string(), base),
    ])
}

fn shell_code_line(th: ThemePalette, line: &str, base: Style) -> Line<'static> {
    let trimmed = line.trim_start();
    if trimmed.starts_with('#') {
        return Line::from(vec![
            Span::raw("  │ "),
            Span::styled(
                line.to_string(),
                Style::default().fg(th.muted).add_modifier(Modifier::ITALIC),
            ),
        ]);
    }
    if let Some(ch) = trimmed.chars().next() {
        if ch == '$' || ch == '>' {
            let indent = line.len().saturating_sub(trimmed.len());
            let prefix = &line[..indent];
            return Line::from(vec![
                Span::raw("  │ "),
                Span::raw(prefix.to_string()),
                Span::styled(
                    ch.to_string(),
                    Style::default().fg(th.accent).add_modifier(Modifier::BOLD),
                ),
                Span::styled(trimmed[1..].to_string(), base),
            ]);
        }
    }
    Line::from(vec![
        Span::raw("  │ "),
        Span::styled(line.to_string(), base),
    ])
}

struct MarkdownRenderer {
    th: ThemePalette,
    base: Style,
    max_width: Option<usize>,
    style_stack: Vec<Style>,
    lines: Vec<Line<'static>>,
    current: Vec<Span<'static>>,
    list_depth: u32,
    ordered_index: u32,
    in_ordered_list: bool,
    link_url: Option<String>,
    in_code_block: bool,
    code_block_lang: Option<String>,
    table: Option<TableBuilder>,
}

impl MarkdownRenderer {
    fn new(th: ThemePalette, base: Style, max_width: Option<usize>) -> Self {
        Self {
            th,
            base,
            max_width,
            style_stack: vec![base],
            lines: Vec::new(),
            current: Vec::new(),
            list_depth: 0,
            ordered_index: 0,
            in_ordered_list: false,
            link_url: None,
            in_code_block: false,
            code_block_lang: None,
            table: None,
        }
    }

    fn in_table_cell(&self) -> bool {
        self.table.as_ref().is_some_and(|t| t.in_cell)
    }

    fn append_table_cell(&mut self, text: &str) -> bool {
        if let Some(table) = &mut self.table {
            if table.in_cell {
                table.current_cell.push_str(text);
                return true;
            }
        }
        false
    }

    fn flush_table_row(&mut self) {
        let Some(table) = &mut self.table else {
            return;
        };
        if table.current_row.is_empty() {
            return;
        }
        if !is_table_separator_row(&table.current_row) {
            table.rows.push(std::mem::take(&mut table.current_row));
        }
        table.current_row.clear();
    }

    fn finish_table(&mut self) {
        self.flush_table_row();
        if let Some(table) = self.table.take() {
            self.lines.extend(format_table_lines(
                self.th,
                table.rows,
                self.base,
                self.max_width,
            ));
            self.new_line();
        }
    }

    fn current_style(&self) -> Style {
        *self.style_stack.last().unwrap_or(&self.base)
    }

    fn on_event(&mut self, event: Event<'_>) {
        match event {
            Event::Start(tag) => self.start_tag(tag),
            Event::End(tag) => self.end_tag(tag),
            Event::Text(text) => self.push_text(text.as_ref()),
            Event::Code(text) => self.push_inline_code(text.as_ref()),
            Event::SoftBreak => {
                if self.in_table_cell() {
                    self.append_table_cell(" ");
                } else {
                    self.push_text(" ");
                }
            }
            Event::HardBreak => {
                if self.in_table_cell() {
                    self.append_table_cell(" ");
                } else {
                    self.new_line();
                }
            }
            Event::Rule => {
                self.flush_line();
                self.lines.push(Line::from(Span::styled(
                    "─".repeat(24),
                    Style::default().fg(self.th.muted),
                )));
            }
            _ => {}
        }
    }

    fn start_tag(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => {}
            Tag::Heading { level, .. } => {
                self.flush_line();
                if !self.lines.is_empty() {
                    self.lines.push(Line::from(""));
                }
                self.style_stack.push(heading_style(self.th, level));
            }
            Tag::Strong => {
                self.style_stack
                    .push(self.current_style().add_modifier(Modifier::BOLD));
            }
            Tag::Emphasis => {
                self.style_stack
                    .push(self.current_style().add_modifier(Modifier::ITALIC));
            }
            Tag::Strikethrough => {
                self.style_stack
                    .push(self.current_style().add_modifier(Modifier::CROSSED_OUT));
            }
            Tag::Link { dest_url, .. } => {
                self.link_url = Some(dest_url.to_string());
                self.style_stack.push(
                    Style::default()
                        .fg(self.th.link)
                        .add_modifier(Modifier::UNDERLINED),
                );
            }
            Tag::List(start) => {
                self.list_depth += 1;
                self.in_ordered_list = start.is_some();
                self.ordered_index = start.unwrap_or(1) as u32;
            }
            Tag::Item => {
                self.flush_line();
                let indent = "  ".repeat(self.list_depth.saturating_sub(1) as usize);
                if self.in_ordered_list {
                    let marker = format!("{indent}{}. ", self.ordered_index);
                    self.ordered_index += 1;
                    self.push_span(
                        marker,
                        Style::default()
                            .fg(self.th.accent)
                            .add_modifier(Modifier::BOLD),
                    );
                } else {
                    self.push_span(format!("{indent}▸ "), Style::default().fg(self.th.warn));
                }
            }
            Tag::CodeBlock(kind) => {
                self.flush_line();
                self.in_code_block = true;
                self.code_block_lang = match kind {
                    CodeBlockKind::Fenced(lang) => {
                        let lang = lang.trim();
                        if lang.is_empty() {
                            None
                        } else {
                            Some(lang.to_ascii_lowercase())
                        }
                    }
                    CodeBlockKind::Indented => None,
                };
                let label = self
                    .code_block_lang
                    .as_deref()
                    .unwrap_or("code");
                self.lines.push(Line::from(Span::styled(
                    format!("  ┌─ {label} "),
                    Style::default().fg(self.th.muted),
                )));
                self.style_stack
                    .push(Style::default().fg(self.th.code_fg).bg(self.th.code_bg));
            }
            Tag::BlockQuote(_) => {
                self.flush_line();
                let indent = "  ".repeat(self.list_depth as usize);
                self.push_span(
                    format!("{indent}▎ "),
                    Style::default()
                        .fg(self.th.muted)
                        .add_modifier(Modifier::ITALIC),
                );
                self.style_stack
                    .push(self.current_style().add_modifier(Modifier::ITALIC));
            }
            Tag::Table(_) => {
                self.flush_line();
                self.table = Some(TableBuilder::new());
            }
            Tag::TableHead | Tag::TableRow => {}
            Tag::TableCell => {
                if let Some(table) = &mut self.table {
                    table.in_cell = true;
                    table.current_cell.clear();
                }
            }
            _ => {}
        }
    }

    fn end_tag(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph if !self.in_table_cell() => self.new_line(),
            TagEnd::Heading(level) => {
                self.style_stack.pop();
                self.flush_line();
                if level == HeadingLevel::H1 {
                    self.lines.push(Line::from(Span::styled(
                        "─".repeat(20),
                        Style::default().fg(self.th.muted),
                    )));
                }
                self.new_line();
            }
            TagEnd::Strong | TagEnd::Emphasis | TagEnd::Strikethrough => {
                self.style_stack.pop();
            }
            TagEnd::Link => {
                if let Some(url) = self.link_url.take() {
                    if self.th.osc8_links && !self.current.is_empty() {
                        let label: String = self
                            .current
                            .iter()
                            .map(|s| s.content.as_ref())
                            .collect();
                        let style = self
                            .current
                            .last()
                            .map(|s| s.style)
                            .unwrap_or_else(|| self.current_style());
                        self.current.clear();
                        self.push_span(osc8_link(&url, &label), style);
                    } else {
                        self.push_span(
                            format!(" ↗ {}", shorten_url(&url)),
                            Style::default().fg(self.th.muted),
                        );
                    }
                }
                self.style_stack.pop();
            }
            TagEnd::List(_) => {
                self.list_depth = self.list_depth.saturating_sub(1);
                if self.list_depth == 0 {
                    self.in_ordered_list = false;
                }
                self.new_line();
            }
            TagEnd::Item => {
                self.new_line();
            }
            TagEnd::CodeBlock => {
                self.style_stack.pop();
                self.in_code_block = false;
                self.code_block_lang = None;
                self.lines.push(Line::from(Span::styled(
                    "  └─",
                    Style::default().fg(self.th.muted),
                )));
                self.new_line();
            }
            TagEnd::BlockQuote(_) => {
                self.style_stack.pop();
                self.new_line();
            }
            TagEnd::TableCell => {
                if let Some(table) = &mut self.table {
                    table.in_cell = false;
                    table
                        .current_row
                        .push(table.current_cell.trim().to_string());
                    table.current_cell.clear();
                }
            }
            TagEnd::TableRow | TagEnd::TableHead => {
                self.flush_table_row();
            }
            TagEnd::Table => {
                self.finish_table();
            }
            _ => {}
        }
    }

    fn push_inline_code(&mut self, text: &str) {
        if self.append_table_cell(text) {
            return;
        }
        self.push_span(
            format!(" `{text}` "),
            Style::default().fg(self.th.code_fg).bg(self.th.code_bg),
        );
    }

    fn push_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        if self.append_table_cell(text) {
            return;
        }
        if self.in_code_block {
            for part in text.split('\n') {
                if !self.current.is_empty() {
                    self.new_line();
                }
                if self.code_block_lang.as_deref() == Some("json") {
                    self.lines
                        .push(json_code_line(self.th, part, self.current_style()));
                } else if matches!(
                    self.code_block_lang.as_deref(),
                    Some("yaml") | Some("yml")
                ) {
                    self.lines
                        .push(yaml_code_line(self.th, part, self.current_style()));
                } else if matches!(
                    self.code_block_lang.as_deref(),
                    Some("sh") | Some("bash") | Some("shell") | Some("zsh")
                ) {
                    self.lines
                        .push(shell_code_line(self.th, part, self.current_style()));
                } else if matches!(self.code_block_lang.as_deref(), Some("go") | Some("golang")) {
                    self.lines
                        .push(go_code_line(self.th, part, self.current_style()));
                } else if matches!(self.code_block_lang.as_deref(), Some("rust") | Some("rs")) {
                    self.lines
                        .push(rust_code_line(self.th, part, self.current_style()));
                } else if matches!(self.code_block_lang.as_deref(), Some("toml")) {
                    self.lines
                        .push(toml_code_line(self.th, part, self.current_style()));
                } else {
                    self.push_span(format!("  │ {part}"), self.current_style());
                }
            }
            return;
        }
        self.push_spans_with_pr_refs(text, self.current_style());
    }

    fn push_spans_with_pr_refs(&mut self, text: &str, style: Style) {
        let mut i = 0usize;
        while i < text.len() {
            if let Some((start, end, kind)) = next_inline_highlight(text, i) {
                if start > i {
                    self.push_span(text[i..start].to_string(), style);
                }
                let slice = highlight_label(text, start, end, kind);
                let hi = match kind {
                    InlineHighlight::Pr => Style::default()
                        .fg(self.th.pr_ref)
                        .add_modifier(Modifier::BOLD),
                    InlineHighlight::Run => Style::default()
                        .fg(self.th.accent)
                        .add_modifier(Modifier::BOLD),
                    InlineHighlight::Repo => Style::default()
                        .fg(self.th.link)
                        .add_modifier(Modifier::UNDERLINED),
                };
                self.push_span(slice, hi);
                i = end;
            } else {
                self.push_span(text[i..].to_string(), style);
                break;
            }
        }
    }

    fn push_span(&mut self, text: String, style: Style) {
        if text.is_empty() {
            return;
        }
        if let Some(last) = self.current.last_mut() {
            if last.style == style {
                last.content.to_mut().push_str(&text);
                return;
            }
        }
        self.current.push(Span::styled(text, style));
    }

    fn new_line(&mut self) {
        self.flush_line();
    }

    fn flush_line(&mut self) {
        if self.current.is_empty() {
            return;
        }
        let line = Line::from(std::mem::take(&mut self.current));
        self.lines.push(enrich_pr_refs(self.th, line));
    }

    fn finish(mut self) -> Vec<Line<'static>> {
        self.finish_table();
        self.flush_line();
        while self
            .lines
            .last()
            .is_some_and(|l| l.spans.is_empty() || l.spans.iter().all(|s| s.content.is_empty()))
        {
            self.lines.pop();
        }
        if self.lines.is_empty() {
            self.lines.push(Line::from(""));
        }
        self.lines
    }
}

fn enrich_pr_refs(th: ThemePalette, mut line: Line<'static>) -> Line<'static> {
    let mut new_spans = Vec::new();
    for span in line.spans {
        let style = span.style;
        let text = span.content.into_owned();
        if span.style.bg == Some(th.code_bg) {
            new_spans.push(Span::styled(text, style));
            continue;
        }
        let mut i = 0usize;
        while i < text.len() {
            if let Some((start, end, kind)) = next_inline_highlight(&text, i) {
                if start > i {
                    new_spans.push(Span::styled(text[i..start].to_string(), style));
                }
                let hi = match kind {
                    InlineHighlight::Pr => Style::default().fg(th.pr_ref).add_modifier(Modifier::BOLD),
                    InlineHighlight::Run => {
                        Style::default().fg(th.accent).add_modifier(Modifier::BOLD)
                    }
                    InlineHighlight::Repo => Style::default()
                        .fg(th.link)
                        .add_modifier(Modifier::UNDERLINED),
                };
                new_spans.push(Span::styled(highlight_label(&text, start, end, kind), hi));
                i = end;
            } else {
                new_spans.push(Span::styled(text[i..].to_string(), style));
                break;
            }
        }
    }
    line.spans = new_spans;
    line
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InlineHighlight {
    Pr,
    Run,
    Repo,
}

fn highlight_label(text: &str, start: usize, end: usize, kind: InlineHighlight) -> String {
    let raw = &text[start..end];
    match kind {
        InlineHighlight::Pr if raw.starts_with('#') => format!("[PR {raw}]"),
        _ => raw.to_string(),
    }
}

fn next_inline_highlight(text: &str, from: usize) -> Option<(usize, usize, InlineHighlight)> {
    let bytes = text.as_bytes();
    let mut i = from;
    while i < bytes.len() {
        if let Some(end) = match_hash_pr(bytes, i) {
            return Some((i, end, InlineHighlight::Pr));
        }
        if let Some(end) = match_pr_label(bytes, i) {
            return Some((i, end, InlineHighlight::Pr));
        }
        if let Some(end) = match_repo_slug(bytes, i) {
            return Some((i, end, InlineHighlight::Repo));
        }
        if let Some(end) = match_run_ref(bytes, i) {
            return Some((i, end, InlineHighlight::Run));
        }
        i += 1;
    }
    None
}

fn match_hash_pr(bytes: &[u8], i: usize) -> Option<usize> {
    if bytes.get(i) != Some(&b'#') {
        return None;
    }
    let mut j = i + 1;
    while j < bytes.len() && bytes[j].is_ascii_digit() {
        j += 1;
    }
    if j > i + 1 {
        Some(j)
    } else {
        None
    }
}

fn match_pr_label(bytes: &[u8], i: usize) -> Option<usize> {
    if i + 4 >= bytes.len() {
        return None;
    }
    if !bytes[i].eq_ignore_ascii_case(&b'p') || !bytes[i + 1].eq_ignore_ascii_case(&b'r') {
        return None;
    }
    if bytes[i + 2] != b' ' || bytes[i + 3] != b'#' {
        return None;
    }
    let mut j = i + 4;
    while j < bytes.len() && bytes[j].is_ascii_digit() {
        j += 1;
    }
    if j > i + 4 {
        Some(j)
    } else {
        None
    }
}

fn is_repo_slug_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.')
}

fn repo_slug_boundary_ok(bytes: &[u8], i: usize) -> bool {
    i == 0 || !matches!(bytes[i - 1], b':' | b'/' | b'.' | b'@')
}

fn match_repo_slug(bytes: &[u8], i: usize) -> Option<usize> {
    if !repo_slug_boundary_ok(bytes, i) {
        return None;
    }
    let start = i;
    let mut j = i;
    while j < bytes.len() && is_repo_slug_char(bytes[j]) {
        j += 1;
    }
    if j == start || bytes.get(j) != Some(&b'/') {
        return None;
    }
    j += 1;
    let seg2 = j;
    while j < bytes.len() && is_repo_slug_char(bytes[j]) {
        j += 1;
    }
    if j <= seg2 || j - start < 3 {
        return None;
    }
    if !bytes[start..j].iter().any(|b| b.is_ascii_alphabetic()) {
        return None;
    }
    Some(j)
}

fn match_run_ref(bytes: &[u8], i: usize) -> Option<usize> {
    if i + 4 >= bytes.len() {
        return None;
    }
    if !(bytes[i].eq_ignore_ascii_case(&b'r')
        && bytes[i + 1].eq_ignore_ascii_case(&b'u')
        && bytes[i + 2].eq_ignore_ascii_case(&b'n'))
    {
        return None;
    }
    let mut j = i + 3;
    if j + 2 < bytes.len()
        && bytes[j] == b'_'
        && bytes[j + 1].eq_ignore_ascii_case(&b'i')
        && bytes[j + 2].eq_ignore_ascii_case(&b'd')
    {
        j += 3;
    }
    while j < bytes.len() && matches!(bytes[j], b' ' | b'#' | b':' | b'=') {
        j += 1;
    }
    let digit_start = j;
    while j < bytes.len() && bytes[j].is_ascii_digit() {
        j += 1;
    }
    if j >= digit_start + 5 {
        Some(j)
    } else {
        None
    }
}

fn shorten_url(url: &str) -> String {
    let stripped = url
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    if stripped.chars().count() > 36 {
        format!("{}…", truncate_chars(stripped, 33))
    } else {
        stripped.to_string()
    }
}

fn osc8_link(url: &str, label: &str) -> String {
    format!("\x1b]8;;{url}\x1b\\{label}\x1b]8;;\x1b\\")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::theme::ThemePalette;
    use ratatui::style::Modifier;

    fn dark() -> ThemePalette {
        ThemePalette::dark()
    }

    fn line_text(line: &Line) -> String {
        line.spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect::<String>()
    }

    #[test]
    fn markdown_grows_with_more_input() {
        let th = dark();
        let base = Style::default().fg(th.text);
        let l1 = markdown_to_lines_in_width(th, "Hello", base, None);
        assert!(!l1.is_empty());
        let l2 = markdown_to_lines_in_width(th, "Hello\n* item", base, None);
        assert!(l2.len() >= l1.len());
    }

    #[test]
    fn json_code_fence_highlights_keys_and_strings() {
        let th = dark();
        let md = "```json\n{\"repo\": \"acme/widget\", \"ok\": true}\n```";
        let lines = markdown_to_lines_in_width(th, md, Style::default().fg(th.text), None);
        let joined: String = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.as_ref())
            .collect();
        assert!(joined.contains("\"repo\""));
        assert!(
            lines.iter().flat_map(|l| &l.spans).any(|s| s.style.fg == Some(th.accent)),
            "expected accent key color"
        );
        assert!(
            lines.iter().flat_map(|l| &l.spans).any(|s| s.style.fg == Some(th.ok)),
            "expected string value color"
        );
    }

    #[test]
    fn yaml_code_fence_highlights_keys() {
        let th = dark();
        let md = "```yaml\nrepos:\n  - acme/widget\n```";
        let lines = markdown_to_lines_in_width(th, md, Style::default().fg(th.text), None);
        assert!(
            lines.iter().flat_map(|l| &l.spans).any(|s| s.style.fg == Some(th.accent)),
            "expected yaml key accent"
        );
    }

    #[test]
    fn shell_code_fence_highlights_comments() {
        let th = dark();
        let md = "```bash\n# install deps\n$ cargo build\n```";
        let lines = markdown_to_lines_in_width(th, md, Style::default().fg(th.text), None);
        let joined: String = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.as_ref())
            .collect();
        assert!(joined.contains('#'));
        assert!(joined.contains('$'));
    }

    #[test]
    fn go_code_fence_highlights_keywords() {
        let th = dark();
        let md = "```go\npackage main\n\nfunc main() {}\n```";
        let lines = markdown_to_lines_in_width(th, md, Style::default().fg(th.text), None);
        let joined: String = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.as_ref())
            .collect();
        assert!(joined.contains("package"));
        assert!(joined.contains("func"));
    }

    #[test]
    fn rust_code_fence_highlights_keywords() {
        let th = dark();
        let md = "```rust\nfn main() {\n    // ok\n}\n```";
        let lines = markdown_to_lines_in_width(th, md, Style::default().fg(th.text), None);
        let joined: String = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.as_ref())
            .collect();
        assert!(joined.contains("fn "));
    }

    #[test]
    fn renders_bold() {
        let th = dark();
        let lines =
            markdown_to_lines_in_width(th, "**PR #1** details", Style::default().fg(th.text), None);
        let joined = line_text(&lines[0]);
        assert!(joined.contains("PR #1"));
        assert!(lines[0]
            .spans
            .iter()
            .any(|s| s.style.add_modifier.contains(Modifier::BOLD)));
    }

    #[test]
    fn renders_bullet_list() {
        let th = dark();
        let md = "* **#1**: foo\n* **#2**: bar";
        let lines = markdown_to_lines_in_width(th, md, Style::default().fg(th.text), None);
        assert!(lines.len() >= 2);
        assert!(lines.iter().any(|l| line_text(l).contains('▸')));
    }

    #[test]
    fn renders_ordered_list() {
        let th = dark();
        let md = "1. first\n2. second";
        let lines = markdown_to_lines_in_width(th, md, Style::default().fg(th.text), None);
        assert!(lines.iter().any(|l| line_text(l).starts_with("1. ")));
    }

    #[test]
    fn renders_link() {
        let th = dark();
        let lines = markdown_to_lines_in_width(
            th,
            "[run](https://github.com/a/b)",
            Style::default().fg(th.text),
            None,
        );
        let joined = line_text(&lines[0]);
        assert!(joined.contains("run"));
        assert!(joined.contains("github.com"));
    }

    #[test]
    fn table_with_bold_heading_renders_columns() {
        let th = dark();
        let md = "**Summary:**\n\n| PR | Status |\n| --- | --- |\n| #1 | open |";
        let lines = markdown_to_lines_in_width(th, md, Style::default().fg(th.text), None);
        let joined: Vec<String> = lines.iter().map(|l| line_text(l)).collect();
        assert!(
            joined.iter().any(|l| l.contains('│')),
            "expected box chars: {joined:?}"
        );
        assert!(
            joined
                .iter()
                .any(|l| l.contains("#1") && l.contains("open")),
            "expected data row: {joined:?}"
        );
        assert!(
            !joined.iter().any(|l| l.contains("PRStatus")),
            "cells must not collapse: {joined:?}"
        );
    }

    #[test]
    fn table_renders_as_structured_rows() {
        let th = dark();
        let md = "| PR | Status |\n| --- | --- |\n| #1 | open |\n| #2 | closed |";
        let lines = markdown_to_lines_in_width(th, md, Style::default().fg(th.text), None);
        let joined: Vec<String> = lines.iter().map(|l| line_text(l)).collect();
        assert!(
            joined.iter().any(|l| l.contains('│')),
            "expected formatted table, got: {joined:?}"
        );
        assert!(
            joined
                .iter()
                .any(|l| l.contains("#2") && l.contains("closed")),
            "expected second row, got: {joined:?}"
        );
        assert!(
            !joined.iter().any(|l| l.contains("---")),
            "separator markdown row should not appear raw, got: {joined:?}"
        );
    }

    #[test]
    fn digest_body_renders_section_headings() {
        let th = dark();
        let md = "## Needs attention\n\n* **#19235** acme/widget — CI failing\n\n## Ignorable\n\n* green runs only";
        let lines = markdown_to_lines_in_width(th, md, Style::default().fg(th.text), None);
        let joined: Vec<String> = lines.iter().map(|l| line_text(l)).collect();
        assert!(
            joined.iter().any(|l| l.contains("Needs attention")),
            "expected section heading: {joined:?}"
        );
        assert!(
            joined.iter().any(|l| l.contains("19235")),
            "expected PR bullet: {joined:?}"
        );
    }

    #[test]
    fn atx_heading_levels_use_distinct_colors() {
        let th = dark();
        let base = Style::default().fg(th.assistant);
        let md = "# Title one\n\n## Title two\n\n### Title three";
        let lines = markdown_to_lines_in_width(th, md, base, None);
        let heading_lines: Vec<_> = lines
            .iter()
            .filter(|l| {
                let t = line_text(l);
                t.contains("Title one") || t.contains("Title two") || t.contains("Title three")
            })
            .collect();
        assert_eq!(
            heading_lines.len(),
            3,
            "expected three heading lines: {lines:?}"
        );
        let colors: Vec<_> = heading_lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.style.fg))
            .collect();
        assert!(
            colors.windows(2).any(|w| w[0] != w[1]),
            "heading levels should differ in color: {colors:?}"
        );
    }

    #[test]
    fn numbered_sections_promoted_to_headings() {
        let th = dark();
        let base = Style::default().fg(th.assistant);
        let md = "1. 代码修改核心内容\n\nA. 新增 `smart_router.py`\n\n✅ 优点\n\n* detail";
        let lines = markdown_to_lines_in_width(th, md, base, None);
        let section = lines
            .iter()
            .find(|l| line_text(l).contains("代码修改核心内容"))
            .expect("section title");
        assert!(
            section
                .spans
                .iter()
                .any(|s| s.style.add_modifier.contains(Modifier::BOLD)),
            "promoted section should be bold: {section:?}"
        );
        assert_ne!(
            section.spans[0].style.fg,
            Some(th.assistant),
            "section title should not use body color"
        );
    }

    #[test]
    fn promote_section_line_helpers() {
        assert_eq!(promote_section_line("1. Overview"), "## 1. Overview");
        assert_eq!(promote_section_line("A. Details"), "### A. Details");
        assert_eq!(promote_section_line("✅ Pros"), "### ✅ Pros");
        assert_eq!(promote_section_line("## Already"), "## Already");
    }

    #[test]
    fn consecutive_numbered_lines_stay_ordered_list() {
        let th = dark();
        let base = Style::default().fg(th.assistant);
        let md = "1. **Detection**: short\n2. **Filtering**: short\n3. **Action**: Instead of running the full suite, it would skip the heavy integration tests and only run the most relevant unit tests or even just documentation checks.";
        let lines = markdown_to_lines_in_width(th, md, base, None);
        let joined: Vec<String> = lines.iter().map(|l| line_text(l)).collect();
        assert!(
            joined.iter().filter(|l| l.starts_with("1. ")).count() >= 1,
            "expected ordered list item 1: {joined:?}"
        );
        assert!(
            joined.iter().filter(|l| l.starts_with("2. ")).count() >= 1,
            "expected ordered list item 2: {joined:?}"
        );
        assert!(
            joined.iter().filter(|l| l.starts_with("3. ")).count() >= 1,
            "expected ordered list item 3: {joined:?}"
        );
        assert!(
            !joined.iter().any(|l| l.starts_with("## 1.")),
            "list items should not be promoted to headings: {joined:?}"
        );
    }

    #[test]
    fn ordered_list_wrap_uses_hang_indent() {
        let th = dark();
        let base = Style::default().fg(th.assistant);
        let md = "1. first\n2. second\n3. **Action**: Instead of running the full suite, it would skip the heavy integration tests and only run unit tests or documentation checks.";
        let width = 52;
        let lines = markdown_to_lines_in_width(th, md, base, Some(width));
        let joined: Vec<String> = lines.iter().map(|l| line_text(l)).collect();
        let item3_lines: Vec<_> = joined
            .iter()
            .skip_while(|l| !l.starts_with("3. "))
            .take_while(|l| l.starts_with(' ') || l.starts_with("3. "))
            .collect();
        assert!(
            item3_lines.len() >= 2,
            "long list item should wrap: {joined:?}"
        );
        let continuation = item3_lines[1];
        assert!(
            continuation.starts_with("   "),
            "wrapped continuation should hang-indent under list text: {continuation:?} in {joined:?}"
        );
        assert!(
            !continuation.trim_start().starts_with("checks."),
            "continuation should be indented, not flush-left: {continuation:?}"
        );
    }

    #[test]
    fn pad_lines_to_panel_width_fills_short_rows() {
        let width = 20u16;
        let lines = vec![Line::from("short")];
        let padded = pad_lines_to_panel_width(lines, width, Style::default());
        assert_eq!(padded.len(), 1);
        assert_eq!(line_display_width_for_test(&padded[0]), width as usize);
    }

    #[test]
    fn tools_table_fits_narrow_width_without_lone_pipes() {
        let th = dark();
        let md = "## MCP\n\n| Tool | When to use |\n|------|-------------|\n\
| `pr_get_overview` | PR snapshot (#N status, CI, file **counts** only) |\n\
| `pr_list_changed_files` | **File list** with +/− line counts |";
        let width = 52;
        let lines = markdown_to_lines_in_width(th, md, Style::default().fg(th.text), Some(width));
        let joined: Vec<String> = lines.iter().map(|l| line_text(l)).collect();
        for line in &joined {
            if line.trim() == "│" {
                panic!("broken table row (lone pipe): {joined:?}");
            }
            if line.contains('│') {
                assert!(
                    UnicodeWidthStr::width(line.as_str()) <= width,
                    "table row wider than panel: {width}w {line:?} in {joined:?}"
                );
            }
        }
        assert!(
            joined.iter().any(|l| l.contains("pr_get_overview")),
            "expected tool row: {joined:?}"
        );
    }

    #[test]
    fn pr_list_open_tool_result_keeps_newlines() {
        let th = dark();
        let body = "tool_result(pr_list_open):\n(list may be truncated at limit=20)\n\
open PR(s) in acme/widget (3):\n\
#19264 backport @x CI:failing(6) review:none\n\
#19263 feat @y CI:passing review:none\n\
#19262 fix @z CI:pending review:none";
        let lines = markdown_to_lines_in_width(th, body, Style::default().fg(th.text), None);
        let texts: Vec<String> = lines.iter().map(|l| line_text(l)).collect();
        assert!(texts.len() >= 5, "expected one line per row, got {texts:?}");
        assert!(
            !texts
                .iter()
                .any(|l| l.contains("19264") && l.contains("19263")),
            "PR rows must not merge into one line: {texts:?}"
        );
        assert!(texts.iter().any(|l| l.contains("#19264")));
        assert!(texts.iter().any(|l| l.contains("#19263")));
    }

    #[test]
    fn truncate_respects_display_width_with_tabs() {
        let s = "  3UNKNOWN STEP\t        Please ensure";
        let out = truncate_to_display_width(s, 30);
        assert!(
            UnicodeWidthStr::width(out.as_str()) <= 30,
            "truncated {out:?} is {}w",
            UnicodeWidthStr::width(out.as_str())
        );
    }

    #[test]
    fn plain_text_wraps_when_max_width_set() {
        let th = dark();
        let long = "alpha ".repeat(40);
        let lines = markdown_to_lines_in_width(th, &long, Style::default().fg(th.text), Some(32));
        assert!(lines.len() > 1, "plain text should pre-wrap to max_width");
    }

    #[test]
    fn table_rows_are_not_split_by_width() {
        let th = dark();
        let md = "| A | B |\n|---|---|\n| long cell value | x |";
        let lines = markdown_to_lines_in_width(th, md, Style::default().fg(th.text), Some(20));
        let table_rows: Vec<_> = lines
            .iter()
            .map(|l| line_text(l))
            .filter(|l| {
                let t = l.trim();
                t.starts_with('│') || (t.starts_with('├') && t.ends_with('┤'))
            })
            .collect();
        assert!(
            table_rows.len() >= 2,
            "expected data table rows, got {table_rows:?}"
        );
        for row in &table_rows {
            assert!(
                !row.contains('\n'),
                "table row must stay on one line: {row:?}"
            );
        }
    }

    #[test]
    fn hash_pr_refs_without_tool_prefix_still_keep_newlines() {
        let th = dark();
        let body = "#19264 a @x CI:failing(6)\n#19263 b @y CI:passing";
        let lines = markdown_to_lines_in_width(th, body, Style::default().fg(th.text), None);
        assert_eq!(lines.len(), 2);
        assert!(line_text(&lines[0]).contains("19264"));
        assert!(line_text(&lines[1]).contains("19263"));
    }

    #[test]
    fn streaming_markdown_caches_complete_lines() {
        let th = dark();
        let base = Style::default().fg(th.text);
        let mut renderer = StreamingMarkdownRenderer::new();
        let l1 = renderer.render(th, "Line one\nLine two", base, None);
        assert_eq!(
            renderer.stable_byte_len(),
            stable_line_prefix_byte_len("Line one\nLine two")
        );
        let l2 = renderer.render(th, "Line one\nLine two\nLine three", base, None);
        assert!(l2.len() >= l1.len());
        assert_eq!(line_text(&l1[0]), line_text(&l2[0]));
        assert!(l2.iter().any(|l| line_text(l).contains("three")));
    }

    #[test]
    fn streaming_markdown_grows_with_partial_line() {
        let th = dark();
        let base = Style::default().fg(th.assistant);
        let mut renderer = StreamingMarkdownRenderer::new();
        let l1 = renderer.render(th, "**Hello", base, None);
        let l2 = renderer.render(th, "**Hello** world", base, None);
        assert!(l2.len() >= l1.len());
        let joined: String = l2.iter().map(|l| line_text(l)).collect();
        assert!(joined.contains("Hello"));
        assert!(joined.contains("world"));
    }

    #[test]
    fn stable_line_prefix_byte_len_at_newline() {
        assert_eq!(stable_line_prefix_byte_len("ab"), 0);
        assert_eq!(stable_line_prefix_byte_len("ab\n"), 3);
        assert_eq!(stable_line_prefix_byte_len("ab\ncd"), 3);
        assert_eq!(stable_line_prefix_byte_len("ab\ncd\n"), 6);
    }

    #[test]
    fn markdown_h2_section_titles_skips_h3() {
        let md = "## Needs attention\n### repo\n## Ignorable";
        let titles = markdown_h2_section_titles(md);
        assert_eq!(titles, vec!["Needs attention", "Ignorable"]);
    }

    #[test]
    fn filter_folded_markdown_sections_hides_body() {
        let md = "## Needs attention\n\n- item\n\n## Ignorable\n\n- skip me";
        let folded: HashSet<String> = ["Ignorable"].into_iter().map(str::to_string).collect();
        let out = filter_folded_markdown_sections(md, &folded, "Z");
        assert!(out.contains("Needs attention"));
        assert!(out.contains("- item"));
        assert!(out.contains("Ignorable"));
        assert!(out.contains("[folded]"));
        assert!(!out.contains("skip me"));
    }

    #[test]
    fn nested_bullet_wrap_uses_marker_hang_indent() {
        let th = dark();
        let base = Style::default().fg(th.assistant);
        let md = "* outer short\n  * nested item with a long description that should wrap under the bullet marker";
        let width = 42;
        let lines = markdown_to_lines_in_width(th, md, base, Some(width));
        let joined: Vec<String> = lines.iter().map(|l| line_text(l)).collect();
        let nested: Vec<_> = joined
            .iter()
            .skip_while(|l| !l.contains('▸') || !l.starts_with("  "))
            .take_while(|l| l.starts_with(' ') || l.contains('▸'))
            .collect();
        assert!(
            nested.len() >= 2,
            "nested item should wrap: {joined:?}"
        );
        assert!(
            nested[1].starts_with("    "),
            "continuation should hang under nested marker (4 cols), got {:?} in {:?}",
            nested[1],
            joined
        );
    }

    #[test]
    fn list_marker_prefix_width_excludes_body() {
        assert_eq!(
            list_marker_prefix_display_width("    ▸ body text"),
            Some(UnicodeWidthStr::width("    ▸ "))
        );
    }

    #[test]
    fn highlights_pr_label_and_run_id() {
        let th = dark();
        let lines = markdown_to_lines_in_width(
            th,
            "PR #42 failed on run 1234567890",
            Style::default().fg(th.text),
            None,
        );
        let joined: String = lines.iter().map(|l| line_text(l)).collect();
        assert!(joined.contains("PR #42"));
        assert!(joined.contains("1234567890"));
        let pr_bold = lines[0]
            .spans
            .iter()
            .any(|s| s.content.as_ref().contains("PR #42") && s.style.fg == Some(th.pr_ref));
        assert!(pr_bold, "PR label should be highlighted: {:?}", lines[0].spans);
    }

    #[test]
    fn hash_pr_renders_short_pr_label() {
        let th = dark();
        let lines = markdown_to_lines_in_width(
            th,
            "see #42 for details",
            Style::default().fg(th.text),
            None,
        );
        let joined: String = lines[0]
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(
            joined.contains("[PR #42]"),
            "expected short PR label, got {joined:?}"
        );
    }

    #[test]
    fn highlights_owner_repo_slug() {
        let th = dark();
        let lines = markdown_to_lines_in_width(
            th,
            "open PR in acme/widget with failing CI",
            Style::default().fg(th.text),
            None,
        );
        assert!(
            lines[0].spans.iter().any(|s| {
                s.content.as_ref().contains("acme/widget") && s.style.fg == Some(th.link)
            }),
            "repo slug should be highlighted: {:?}",
            lines[0].spans
        );
    }

    #[test]
    fn osc8_links_wrap_markdown_href() {
        let mut th = dark();
        th.osc8_links = true;
        let lines = markdown_to_lines_in_width(
            th,
            "[run](https://github.com/acme/widget/actions/runs/1)",
            Style::default().fg(th.text),
            None,
        );
        let joined: String = lines[0]
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(
            joined.contains("\x1b]8;;https://github.com/acme/widget/actions/runs/1\x1b\\"),
            "expected OSC 8 link wrapper: {joined:?}"
        );
    }
}
