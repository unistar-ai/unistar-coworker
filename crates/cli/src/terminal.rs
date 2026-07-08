use std::io::IsTerminal;
use std::sync::atomic::{AtomicBool, Ordering};

use pulldown_cmark::{Event, Parser as MdParser, Tag, TagEnd};

static PLAIN: AtomicBool = AtomicBool::new(false);

pub(crate) fn set_plain(p: bool) {
    PLAIN.store(p, Ordering::Relaxed);
}
pub(crate) fn use_color_stdout() -> bool {
    std::io::stdout().is_terminal() && !PLAIN.load(Ordering::Relaxed)
}
fn use_color_stderr() -> bool {
    std::io::stderr().is_terminal() && !PLAIN.load(Ordering::Relaxed)
}

/// Wrap `s` in an ANSI SGR sequence when `tty`, else return it untouched.
fn ansi(seq: &str, s: &str, tty: bool) -> String {
    if tty {
        format!("\x1b[{seq}m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}
pub(crate) fn cyan(s: &str, tty: bool) -> String {
    ansi("36", s, tty)
}
pub(crate) fn green(s: &str, tty: bool) -> String {
    ansi("32", s, tty)
}
pub(crate) fn red(s: &str, tty: bool) -> String {
    ansi("31", s, tty)
}
pub(crate) fn yellow(s: &str, tty: bool) -> String {
    ansi("33", s, tty)
}
pub(crate) fn purple(s: &str, tty: bool) -> String {
    ansi("35", s, tty)
}
pub(crate) fn dim(s: &str, tty: bool) -> String {
    ansi("2", s, tty)
}
pub(crate) fn bold(s: &str, tty: bool) -> String {
    ansi("1", s, tty)
}

/// Display width that treats CJK-range codepoints as width 2 (no extra deps).
fn disp_width(s: &str) -> usize {
    s.chars()
        .map(|c| if (c as u32) >= 0x1100 { 2 } else { 1 })
        .sum()
}

/// A box-drawing table. On a TTY renders `┌─┬─┐` borders with aligned columns;
/// otherwise degrades to tab-separated rows (script-friendly).
pub(crate) fn table(headers: &[&str], rows: &[Vec<String>], tty: bool) -> String {
    if !tty {
        let mut out = String::new();
        out.push_str(&headers.join("\t"));
        out.push('\n');
        for r in rows {
            out.push_str(&r.join("\t"));
            out.push('\n');
        }
        return out;
    }
    let cols = headers.len();
    let mut widths: Vec<usize> = headers.iter().map(|h| disp_width(h)).collect();
    for r in rows {
        for (i, c) in r.iter().enumerate() {
            if i < cols {
                widths[i] = widths[i].max(disp_width(c));
            }
        }
    }
    let mut out = String::new();
    out.push_str(&hbar(&widths, '┌', '┬', '┐'));
    out.push_str(&row_line(headers, &widths, true));
    out.push_str(&hbar(&widths, '├', '┼', '┤'));
    for r in rows {
        out.push_str(&row_line(r, &widths, false));
    }
    out.push_str(&hbar(&widths, '└', '┴', '┘'));
    out
}

fn row_line(cells: &[impl AsRef<str>], widths: &[usize], header: bool) -> String {
    let mut s = String::from("│ ");
    for (i, w) in widths.iter().enumerate() {
        let cell = cells.get(i).map(|c| c.as_ref()).unwrap_or("");
        let padded = format!("{:<width$}", cell, width = w);
        if header {
            s.push_str(&bold(&padded, true));
        } else {
            s.push_str(&padded);
        }
        s.push_str(" │ ");
    }
    s.push('\n');
    s
}

fn hbar(widths: &[usize], l: char, m: char, r: char) -> String {
    let mut s = String::from(l);
    for (i, w) in widths.iter().enumerate() {
        s.push_str(&"─".repeat(w + 2));
        s.push(if i + 1 == widths.len() { r } else { m });
    }
    s.push('\n');
    s
}

/// A titled box panel for a short block of text.
pub(crate) fn panel(title: &str, body: &str, tty: bool) -> String {
    if !tty {
        return format!("{title}\n{body}\n");
    }
    let lines: Vec<&str> = body.lines().collect();
    let inner_w = lines
        .iter()
        .map(|l| disp_width(l))
        .max()
        .unwrap_or(0)
        .max(disp_width(title))
        .max(8);
    let mut s = String::new();
    let title_pad = inner_w.saturating_sub(disp_width(title));
    s.push('┌');
    s.push_str(&format!("─ {title}"));
    s.push_str(&"─".repeat(title_pad + 1));
    s.push('┐');
    s.push('\n');
    for l in lines {
        s.push_str(&format!("│ {:<width$} │\n", l, width = inner_w));
    }
    s.push('└');
    s.push_str(&"─".repeat(inner_w + 2));
    s.push('┘');
    s.push('\n');
    s
}

/// ANSI percentage bar: `███░░░ 42%`. Plain mode returns `42%`.
pub(crate) fn progress_bar(pct: f64, width: usize, tty: bool) -> String {
    if !tty {
        return format!("{pct:.0}%");
    }
    let pct = pct.clamp(0.0, 100.0);
    let filled = ((pct / 100.0) * width as f64).round() as usize;
    let filled = filled.min(width);
    let bar: String = "█".repeat(filled);
    let empty: String = "░".repeat(width - filled);
    format!("{}{} {:.0}%", green(&bar, true), dim(&empty, true), pct)
}

const SPINNER: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
pub(crate) fn spinner_frame(i: u64) -> char {
    SPINNER[(i as usize) % SPINNER.len()]
}

/// Pretty-print a JSON value to stdout with ANSI syntax highlighting (TTY only).
/// Non-TTY (or `--plain`) emits a single compact line for piping.
pub(crate) fn emit_json(v: serde_json::Value) {
    if use_color_stdout() {
        println!("{}", highlight_json(&v, 0));
    } else {
        println!(
            "{}",
            serde_json::to_string(&v).unwrap_or_else(|_| v.to_string())
        );
    }
}

fn highlight_json(v: &serde_json::Value, indent: usize) -> String {
    let pad = "  ".repeat(indent);
    let pad1 = "  ".repeat(indent + 1);
    match v {
        serde_json::Value::Object(map) => {
            if map.is_empty() {
                return "{}".to_string();
            }
            let mut s = String::from("{\n");
            for (i, (k, val)) in map.iter().enumerate() {
                s.push_str(&pad1);
                s.push_str(&cyan(&format!("\"{k}\""), true));
                s.push_str(": ");
                s.push_str(&highlight_json(val, indent + 1));
                if i + 1 < map.len() {
                    s.push(',');
                }
                s.push('\n');
            }
            s.push_str(&pad);
            s.push('}');
            s
        }
        serde_json::Value::Array(arr) => {
            if arr.is_empty() {
                return "[]".to_string();
            }
            let mut s = String::from("[\n");
            for (i, val) in arr.iter().enumerate() {
                s.push_str(&pad1);
                s.push_str(&highlight_json(val, indent + 1));
                if i + 1 < arr.len() {
                    s.push(',');
                }
                s.push('\n');
            }
            s.push_str(&pad);
            s.push(']');
            s
        }
        serde_json::Value::String(x) => green(&format!("\"{x}\""), true),
        serde_json::Value::Number(n) => yellow(&n.to_string(), true),
        serde_json::Value::Bool(b) => purple(&b.to_string(), true),
        serde_json::Value::Null => dim("null", true),
    }
}

/// `error:` prefix, red on a TTY (respects `--plain`).
pub fn err_prefix() -> String {
    red("error:", use_color_stderr())
}

/// `warning:` prefix, yellow on a TTY (respects `--plain`).
pub(crate) fn warn_prefix() -> String {
    yellow("warning:", use_color_stderr())
}

/// `hint:` prefix, cyan on a TTY (respects `--plain`).
pub(crate) fn hint_prefix() -> String {
    cyan("hint:", use_color_stderr())
}

/// `⏱ timeout:` prefix, yellow on a TTY (respects `--plain`).
pub(crate) fn timeout_prefix() -> String {
    if use_color_stderr() {
        format!("{} timeout:", yellow("⏱", true))
    } else {
        "timeout:".to_string()
    }
}

/// Colorize a `ChatProgress::display_line()` for the terminal: the leading
/// marker (`→` cyan, `✓` green, `✗` red, `⚠`/`⏳` yellow) is colored and the
/// remainder dimmed. Plain text when stderr is not a TTY.
pub(crate) fn colorize_progress(line: &str, tty: bool) -> String {
    if !tty {
        return line.to_string();
    }
    let indent_len = line.len() - line.trim_start().len();
    let indent = &line[..indent_len];
    let rest = &line[indent_len..];
    let (marker, after) = match rest.chars().next() {
        Some('→') => ("\x1b[36m→\x1b[0m", &rest['→'.len_utf8()..]),
        Some('✓') => ("\x1b[32m✓\x1b[0m", &rest['✓'.len_utf8()..]),
        Some('✗') => ("\x1b[31m✗\x1b[0m", &rest['✗'.len_utf8()..]),
        Some('⚠') => ("\x1b[33m⚠\x1b[0m", &rest['⚠'.len_utf8()..]),
        Some('⏳') => ("\x1b[33m⏳\x1b[0m", &rest['⏳'.len_utf8()..]),
        _ => return format!("\x1b[2m{line}\x1b[0m"),
    };
    format!("{indent}{marker}\x1b[2m{after}\x1b[0m")
}

/// P1-3: opening line of a tool-call block (rendered on stderr during a chat
/// turn, separating tool activity from the streamed reply).
pub(crate) fn tool_block_start(name: &str, args_short: &str, tty: bool) -> String {
    if !tty {
        return format!("┌ tool: {name}{}", opt_args(args_short));
    }
    format!(
        "{} {} {}",
        cyan("┌ tool:", true),
        cyan(name, true),
        dim(opt_args(args_short).as_str(), true)
    )
}

/// P1-3: closing line of a tool-call block.
pub(crate) fn tool_block_done(
    name: &str,
    args_short: &str,
    ok: bool,
    elapsed_ms: u128,
    tty: bool,
) -> String {
    let mark = if ok {
        green("✓", tty)
    } else {
        red("✗", tty)
    };
    if !tty {
        return format!(
            "└ {} {}{} ({}ms)",
            mark,
            name,
            opt_args(args_short),
            elapsed_ms
        );
    }
    format!(
        "{} {} {} {}",
        dim("└", true),
        mark,
        cyan(name, true),
        dim(&format!("({}ms){}", elapsed_ms, opt_args(args_short)), true)
    )
}

fn opt_args(args_short: &str) -> String {
    if args_short.is_empty() {
        String::new()
    } else {
        format!("({args_short})")
    }
}

/// One-line tail preview of accumulated reasoning text (newlines → spaces),
/// capped to `max_chars` with a leading `…`. Used for the in-place CLI status.
pub(crate) fn reasoning_tail(text: &str, max_chars: usize) -> String {
    let flat: String = text
        .chars()
        .map(|c| {
            if c.is_control() || c == '\n' || c == '\r' || c == '\t' {
                ' '
            } else {
                c
            }
        })
        .collect();
    let trimmed = flat.trim();
    let chars: Vec<char> = trimmed.chars().collect();
    if chars.len() <= max_chars {
        return trimmed.to_string();
    }
    let tail: String = chars[chars.len() - max_chars..].iter().collect();
    format!("…{tail}")
}

/// Lightweight Markdown → terminal renderer (ANSI). Best-effort: code blocks
/// (indented, dim), inline code (dim), bold, emphasis, headings (bold cyan),
/// list bullets, rules. Falls back to plain text when stdout is not a TTY.
pub(crate) fn render_markdown(text: &str, tty: bool) -> String {
    if !tty || text.trim().is_empty() {
        return text.to_string();
    }
    let mut out = String::new();
    let mut in_code = false;
    let mut code_buf = String::new();
    let mut list_depth: usize = 0;
    for event in MdParser::new(text) {
        match event {
            Event::Start(Tag::CodeBlock(_)) => {
                in_code = true;
                code_buf.clear();
            }
            Event::End(TagEnd::CodeBlock) => {
                in_code = false;
                out.push_str("\x1b[2m");
                for line in code_buf.trim_end_matches('\n').split('\n') {
                    out.push_str("  ");
                    out.push_str(line);
                    out.push('\n');
                }
                out.push_str("\x1b[0m");
            }
            Event::Text(t) if in_code => code_buf.push_str(&t),
            Event::Start(Tag::Heading { .. }) => out.push_str("\x1b[1;36m"),
            Event::End(TagEnd::Heading(_)) => out.push_str("\x1b[0m\n"),
            Event::End(TagEnd::Paragraph) => out.push('\n'),
            Event::Start(Tag::List(_)) => list_depth += 1,
            Event::End(TagEnd::List(_)) => list_depth = list_depth.saturating_sub(1),
            Event::Start(Tag::Item) => {
                for _ in 0..list_depth.saturating_sub(1) {
                    out.push_str("  ");
                }
                out.push_str("• ");
            }
            Event::End(TagEnd::Item) => out.push('\n'),
            Event::Start(Tag::Strong) => out.push_str("\x1b[1m"),
            Event::End(TagEnd::Strong) => out.push_str("\x1b[22m"),
            Event::Start(Tag::Emphasis) => out.push_str("\x1b[3m"),
            Event::End(TagEnd::Emphasis) => out.push_str("\x1b[23m"),
            Event::Code(c) => out.push_str(&format!("\x1b[2m{c}\x1b[22m")),
            Event::Text(t) => out.push_str(&t),
            Event::SoftBreak | Event::HardBreak => out.push('\n'),
            Event::Rule => out.push_str("\x1b[2m────────\x1b[0m\n"),
            _ => {}
        }
    }
    let trimmed = out.trim_end();
    let mut s = trimmed.to_string();
    s.push('\n');
    s
}
