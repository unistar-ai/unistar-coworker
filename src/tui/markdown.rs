use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
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
    if should_preserve_line_breaks(&input) || !looks_like_markdown(&input) {
        return plain_lines(th, &input, base);
    }

    let mut renderer = MarkdownRenderer::new(th, base, max_width);
    let opts = Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES;
    for event in Parser::new_ext(&input, opts) {
        renderer.on_event(event);
    }
    renderer.finish()
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
                let style = match level {
                    pulldown_cmark::HeadingLevel::H1 => Style::default()
                        .fg(self.th.heading_h1)
                        .add_modifier(Modifier::BOLD),
                    pulldown_cmark::HeadingLevel::H2 => Style::default()
                        .fg(self.th.heading_h2)
                        .add_modifier(Modifier::BOLD),
                    _ => Style::default()
                        .fg(self.th.accent_dim)
                        .add_modifier(Modifier::BOLD),
                };
                self.style_stack.push(style);
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
            TagEnd::Heading { .. } => {
                self.style_stack.pop();
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
