use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::agent::context::truncate_chars;

use super::theme::ThemePalette;

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
        return plain_lines(th, &input, base);
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
                enrich_pr_refs(th, Line::from(Span::styled(line.to_string(), base)))
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
    t.chars()
        .nth(1)
        .is_some_and(|c| c.is_ascii_digit())
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

fn truncate_to_display_width(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    if UnicodeWidthStr::width(s) <= max {
        return s.to_string();
    }
    let mut out = String::new();
    let mut used = 0;
    for ch in s.chars() {
        let cw = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + cw > max.saturating_sub(1) {
            out.push('…');
            return out;
        }
        used += cw;
        out.push(ch);
    }
    out
}

#[derive(Clone, Copy)]
struct StyledChar {
    ch: char,
    style: Style,
}

fn line_display_width(line: &Line<'_>) -> usize {
    line.spans
        .iter()
        .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
        .sum()
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

fn is_list_marker_span(text: &str) -> bool {
    if text.contains('▸') {
        return true;
    }
    let trimmed = text.trim_start();
    let bytes = trimmed.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    i > 0 && bytes.get(i) == Some(&b'.') && bytes.get(i + 1) == Some(&b' ')
}

fn list_marker_hang_width(line: &Line<'_>) -> Option<usize> {
    let first = line.spans.first()?;
    if is_list_marker_span(first.content.as_ref()) {
        Some(UnicodeWidthStr::width(first.content.as_ref()))
    } else {
        None
    }
}

fn should_preserve_line_as_single(line: &Line<'_>) -> bool {
    if line.spans.is_empty() {
        return true;
    }
    let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
    text.contains('│') || text.starts_with("  ┌─") || text.starts_with("  └─") || text.starts_with('─')
}

fn wrap_rendered_lines(lines: Vec<Line<'static>>, max_width: Option<usize>) -> Vec<Line<'static>> {
    let Some(width) = max_width.filter(|w| *w > 0) else {
        return lines;
    };
    lines
        .into_iter()
        .flat_map(|line| {
            if should_preserve_line_as_single(&line) || line_display_width(&line) <= width {
                vec![line]
            } else {
                let hang = list_marker_hang_width(&line);
                wrap_line_to_width(line, width, hang)
            }
        })
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
        .unwrap_or(Style::default());

    let mut lines_out: Vec<Vec<StyledChar>> = vec![vec![]];
    let mut col = 0usize;
    let mut budget = max_width;

    let start_new_line = |lines_out: &mut Vec<Vec<StyledChar>>, col: &mut usize, budget: &mut usize| {
        lines_out.push(Vec::new());
        *col = 0;
        *budget = max_width;
        if hang > 0 {
            for _ in 0..hang {
                lines_out
                    .last_mut()
                    .expect("line vec")
                    .push(StyledChar {
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
                if col + cw > budget && col > hang {
                    start_new_line(&mut lines_out, &mut col, &mut budget);
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
            spans.push(Span::styled(
                format!(" {cell}{} ", " ".repeat(pad)),
                base,
            ));
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
                    self.push_span(
                        format!("{indent}▸ "),
                        Style::default().fg(self.th.warn),
                    );
                }
            }
            Tag::CodeBlock(_) => {
                self.flush_line();
                self.in_code_block = true;
                self.lines.push(Line::from(Span::styled(
                    "  ┌─ code ",
                    Style::default().fg(self.th.muted),
                )));
                self.style_stack.push(
                    Style::default()
                        .fg(self.th.code_fg)
                        .bg(self.th.code_bg),
                );
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
                    self.push_span(
                        format!(" ↗ {}", shorten_url(&url)),
                        Style::default().fg(self.th.muted),
                    );
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
            Style::default()
                .fg(self.th.code_fg)
                .bg(self.th.code_bg),
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
                self.push_span(format!("  │ {part}"), self.current_style());
            }
            return;
        }
        self.push_spans_with_pr_refs(text, self.current_style());
    }

    fn push_spans_with_pr_refs(&mut self, text: &str, style: Style) {
        let bytes = text.as_bytes();
        let mut i = 0;
        let mut chunk_start = 0;
        while i < bytes.len() {
            if bytes[i] == b'#' {
                let hash_start = i;
                i += 1;
                let num_start = i;
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
                if i > num_start {
                    if chunk_start < hash_start {
                        self.push_span(text[chunk_start..hash_start].to_string(), style);
                    }
                    self.push_span(
                        text[hash_start..i].to_string(),
                        Style::default()
                            .fg(self.th.pr_ref)
                            .add_modifier(Modifier::BOLD),
                    );
                    chunk_start = i;
                    continue;
                }
                i = hash_start + 1;
            } else {
                i += 1;
            }
        }
        if chunk_start < text.len() {
            self.push_span(text[chunk_start..].to_string(), style);
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
        while self.lines.last().is_some_and(|l| {
            l.spans.is_empty() || l.spans.iter().all(|s| s.content.is_empty())
        }) {
            self.lines.pop();
        }
        if self.lines.is_empty() {
            self.lines.push(Line::from(""));
        }
        self.lines
    }
}

fn enrich_pr_refs(th: ThemePalette, mut line: Line<'static>) -> Line<'static> {
    // Re-highlight #NNN that weren't split during streaming (e.g. plain lines).
    let mut new_spans = Vec::new();
    for span in line.spans {
        let style = span.style;
        let text = span.content.into_owned();
        if text.contains('#') && span.style.bg != Some(th.code_bg) {
            let bytes = text.as_bytes();
            let mut i = 0;
            let mut chunk_start = 0;
            while i < bytes.len() {
                if bytes[i] == b'#' {
                    let hash_start = i;
                    i += 1;
                    let num_start = i;
                    while i < bytes.len() && bytes[i].is_ascii_digit() {
                        i += 1;
                    }
                    if i > num_start {
                        if chunk_start < hash_start {
                            new_spans.push(Span::styled(
                                text[chunk_start..hash_start].to_string(),
                                style,
                            ));
                        }
                        new_spans.push(Span::styled(
                            text[hash_start..i].to_string(),
                            Style::default()
                                .fg(th.pr_ref)
                                .add_modifier(Modifier::BOLD),
                        ));
                        chunk_start = i;
                        continue;
                    }
                    i = hash_start + 1;
                } else {
                    i += 1;
                }
            }
            if chunk_start < text.len() {
                new_spans.push(Span::styled(text[chunk_start..].to_string(), style));
            }
        } else {
            new_spans.push(Span::styled(text, style));
        }
    }
    line.spans = new_spans;
    line
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
    fn renders_bold() {
        let th = dark();
        let lines = markdown_to_lines_in_width(th, "**PR #1** details", Style::default().fg(th.text), None);
        let joined = line_text(&lines[0]);
        assert!(joined.contains("PR #1"));
        assert!(lines[0].spans.iter().any(|s| s.style.add_modifier.contains(Modifier::BOLD)));
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
        assert!(joined.iter().any(|l| l.contains('│')), "expected box chars: {joined:?}");
        assert!(
            joined.iter().any(|l| l.contains("#1") && l.contains("open")),
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
            joined.iter().any(|l| l.contains("#2") && l.contains("closed")),
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
                t.contains("Title one")
                    || t.contains("Title two")
                    || t.contains("Title three")
            })
            .collect();
        assert_eq!(heading_lines.len(), 3, "expected three heading lines: {lines:?}");
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
        assert_eq!(
            promote_section_line("1. Overview"),
            "## 1. Overview"
        );
        assert_eq!(
            promote_section_line("A. Details"),
            "### A. Details"
        );
        assert_eq!(
            promote_section_line("✅ Pros"),
            "### ✅ Pros"
        );
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
            !texts.iter().any(|l| l.contains("19264") && l.contains("19263")),
            "PR rows must not merge into one line: {texts:?}"
        );
        assert!(texts.iter().any(|l| l.contains("#19264")));
        assert!(texts.iter().any(|l| l.contains("#19263")));
    }

    #[test]
    fn plain_text_stays_one_logical_line_for_paragraph_wrap() {
        let th = dark();
        let long = "alpha ".repeat(40);
        let lines = markdown_to_lines_in_width(
            th,
            &long,
            Style::default().fg(th.text),
            Some(32),
        );
        assert_eq!(lines.len(), 1, "non-table text should not be pre-wrapped");
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
}
