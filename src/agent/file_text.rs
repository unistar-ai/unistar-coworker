//! UTF-8 text detection, line endings, and edit matching (Cursor / Aider-style).

use std::path::Path;

use crate::error::{CoworkerError, Result};

const BINARY_SNIFF_BYTES: usize = 8_192;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEnding {
    Lf,
    CrLf,
    Mixed,
}

impl LineEnding {
    pub fn label(self) -> &'static str {
        match self {
            Self::Lf => "lf",
            Self::CrLf => "crlf",
            Self::Mixed => "mixed",
        }
    }
}

#[derive(Debug, Clone)]
pub struct TextFile {
    pub content: String,
    pub line_ending: LineEnding,
    pub had_trailing_newline: bool,
}

/// Read a workspace file as UTF-8 text; reject binary payloads.
pub fn read_text_file(path: &Path) -> Result<TextFile> {
    let bytes =
        std::fs::read(path).map_err(|e| CoworkerError::Workflow(format!("read failed: {e}")))?;
    decode_utf8_text(&bytes)
}

pub fn decode_utf8_text(bytes: &[u8]) -> Result<TextFile> {
    if is_probably_binary(bytes) {
        return Err(CoworkerError::Workflow(
            "file appears binary (NUL byte or invalid UTF-8) — use bash_run for binary files"
                .into(),
        ));
    }
    let content = std::str::from_utf8(bytes)
        .map_err(|_| {
            CoworkerError::Workflow(
                "file is not valid UTF-8 — edit_file/write_file only support UTF-8 text".into(),
            )
        })?
        .to_string();
    Ok(TextFile {
        had_trailing_newline: content.ends_with('\n'),
        line_ending: detect_line_ending(&content),
        content,
    })
}

/// Heuristic: NUL in sniff window, or invalid UTF-8 with high control-char ratio.
pub fn is_probably_binary(bytes: &[u8]) -> bool {
    if bytes.is_empty() {
        return false;
    }
    let sniff = &bytes[..bytes.len().min(BINARY_SNIFF_BYTES)];
    if sniff.contains(&0) {
        return true;
    }
    std::str::from_utf8(sniff).is_err()
}

pub fn detect_line_ending(text: &str) -> LineEnding {
    if text.is_empty() {
        return LineEnding::Lf;
    }
    let crlf_count = text.matches("\r\n").count();
    let lf_count = text.chars().filter(|&c| c == '\n').count();
    let lone_lf = lf_count.saturating_sub(crlf_count);
    match (crlf_count, lone_lf) {
        (0, _) => LineEnding::Lf,
        (_, 0) => LineEnding::CrLf,
        _ => LineEnding::Mixed,
    }
}

/// Normalize `new_string` line endings to match the file (helps CRLF repos).
pub fn normalize_line_endings_for_file(text: &str, ending: LineEnding) -> String {
    let unified = text.replace("\r\n", "\n");
    match ending {
        LineEnding::Lf => unified,
        LineEnding::CrLf => unified.replace('\n', "\r\n"),
        LineEnding::Mixed => text.to_string(),
    }
}

/// Preserve POSIX trailing newline when the original file had one.
pub fn preserve_trailing_newline(original_had: bool, text: &str) -> String {
    if original_had && !text.is_empty() && !text.ends_with('\n') {
        format!("{text}\n")
    } else {
        text.to_string()
    }
}

/// Build old_string variants: exact, then CRLF/LF-normalized (agent often copies from read_file).
pub fn old_string_candidates(old: &str, file: &TextFile) -> Vec<String> {
    let mut out = Vec::new();
    let mut push = |s: String| {
        if !s.is_empty() && !out.contains(&s) {
            out.push(s);
        }
    };
    push(old.to_string());
    push(old.replace("\r\n", "\n"));
    push(old.replace('\n', "\r\n"));
    if file.line_ending == LineEnding::CrLf {
        push(normalize_line_endings_for_file(old, LineEnding::CrLf));
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchMode {
    Unique,
    All,
    NotFound,
    Ambiguous,
}

/// Find how `candidates` match in `haystack`.
pub fn match_old_string(
    haystack: &str,
    candidates: &[String],
    replace_all: bool,
) -> Result<(String, MatchMode, usize)> {
    for candidate in candidates {
        let count = haystack.matches(candidate.as_str()).count();
        if count == 0 {
            continue;
        }
        if replace_all {
            return Ok((candidate.clone(), MatchMode::All, count));
        }
        if count == 1 {
            return Ok((candidate.clone(), MatchMode::Unique, 1));
        }
        return Ok((candidate.clone(), MatchMode::Ambiguous, count));
    }
    Ok((String::new(), MatchMode::NotFound, 0))
}

pub fn apply_replacement(
    original: &str,
    matched: &str,
    new_string: &str,
    mode: MatchMode,
) -> String {
    match mode {
        MatchMode::Unique => original.replacen(matched, new_string, 1),
        MatchMode::All => original.replace(matched, new_string),
        _ => original.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_crlf() {
        assert_eq!(detect_line_ending("a\r\nb\r\n"), LineEnding::CrLf);
        assert_eq!(detect_line_ending("a\nb\n"), LineEnding::Lf);
    }

    #[test]
    fn binary_rejects_nul() {
        assert!(is_probably_binary(b"hello\0world"));
    }

    #[test]
    fn crlf_match_candidate_finds_lf_old_string() {
        let file = TextFile {
            content: "a\r\nb\r\n".into(),
            line_ending: LineEnding::CrLf,
            had_trailing_newline: true,
        };
        let candidates = old_string_candidates("a\nb\n", &file);
        let (m, mode, _) = match_old_string(&file.content, &candidates, false).unwrap();
        assert_eq!(mode, MatchMode::Unique);
        assert!(m.contains('\r'));
    }

    #[test]
    fn preserve_trailing_newline_adds_when_missing() {
        assert_eq!(preserve_trailing_newline(true, "x"), "x\n");
        assert_eq!(preserve_trailing_newline(false, "x"), "x");
    }
}
