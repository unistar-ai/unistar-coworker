//! Local file tools for coding chat (read / search / edit under `chat.workspace`).

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

use crate::agent::context::truncate_chars;
use crate::agent::file_text::{
    apply_replacement, match_old_string, old_string_candidates, normalize_line_endings_for_file,
    preserve_trailing_newline, read_text_file, MatchMode, TextFile,
};
use crate::agent::harness_errors::file_tool_workflow_error;
use crate::error::{CoworkerError, Result};

pub const READ_FILE: &str = "read_file";
pub const GREP: &str = "grep";
pub const GLOB: &str = "glob";
pub const EDIT_FILE: &str = "edit_file";
pub const WRITE_FILE: &str = "write_file";

const READ_FILE_MAX_OUTPUT_CHARS: usize = 32_000;
const READ_FILE_DEFAULT_MAX_LINES: u32 = 500;
const GREP_MAX_MATCHES: usize = 200;
const GREP_MAX_OUTPUT_CHARS: usize = 16_000;
const GLOB_MAX_FILES: usize = 200;
const GLOB_MAX_OUTPUT_CHARS: usize = 8_000;

pub fn is_file_tool(name: &str) -> bool {
    matches!(name, READ_FILE | GREP | GLOB | EDIT_FILE | WRITE_FILE)
}

pub fn is_mutating_file_tool(name: &str) -> bool {
    matches!(name, EDIT_FILE | WRITE_FILE)
}

fn file_err(tool: &str, code: &str, message: impl std::fmt::Display, hint: &str) -> CoworkerError {
    file_tool_workflow_error(tool, code, &message.to_string(), hint)
}

fn load_text_file(tool: &str, path: &Path) -> Result<TextFile> {
    read_text_file(path).map_err(|e| match e {
        CoworkerError::Workflow(msg)
            if msg.contains("binary") || msg.contains("UTF-8") =>
        {
            file_err(
                tool,
                "FILE_BINARY",
                msg,
                "Use bash_run for binary files; edit_file/write_file are UTF-8 text only",
            )
        }
        CoworkerError::Workflow(msg) => {
            file_err(tool, "FILE_IO_ERROR", msg, "Check path and permissions")
        }
        other => other,
    })
}

fn preserve_file_eof(original: &TextFile, mut updated: String) -> String {
    if original.had_trailing_newline && !updated.is_empty() && !updated.ends_with('\n') {
        updated.push('\n');
    }
    updated
}

/// Resolve `user_path` under `workspace`; reject `..` escape after canonicalize.
pub fn resolve_workspace_path(workspace: &Path, user_path: &str) -> Result<PathBuf> {
    let trimmed = user_path.trim();
    if trimmed.is_empty() {
        return Err(file_err(
            READ_FILE,
            "FILE_PATH_EMPTY",
            "path must be non-empty",
            "Pass a workspace-relative path",
        ));
    }
    if trimmed.contains("..") {
        return Err(file_err(
            READ_FILE,
            "FILE_PATH_ESCAPE",
            "path must not contain '..'",
            "Use paths relative to workspace root",
        ));
    }
    let workspace = workspace
        .canonicalize()
        .unwrap_or_else(|_| workspace.to_path_buf());
    let candidate = if Path::new(trimmed).is_absolute() {
        PathBuf::from(trimmed)
    } else {
        workspace.join(trimmed)
    };
    let canonical = if candidate.exists() {
        candidate
            .canonicalize()
            .map_err(|e| {
                file_err(
                    READ_FILE,
                    "FILE_IO_ERROR",
                    format!("could not resolve path: {e}"),
                    "Check path exists under workspace",
                )
            })?
    } else {
        let parent = candidate.parent().ok_or_else(|| {
            file_err(READ_FILE, "FILE_IO_ERROR", "invalid path", "Pass a valid file path")
        })?;
        let file_name = candidate.file_name().ok_or_else(|| {
            file_err(READ_FILE, "FILE_IO_ERROR", "invalid path", "Pass a valid file path")
        })?;
        let parent_canonical = parent.canonicalize().map_err(|e| {
            file_err(
                READ_FILE,
                "FILE_IO_ERROR",
                format!("could not resolve parent: {e}"),
                "Create parent directory or fix path",
            )
        })?;
        parent_canonical.join(file_name)
    };
    if !canonical.starts_with(&workspace) {
        return Err(file_err(
            READ_FILE,
            "FILE_PATH_ESCAPE",
            format!("path {:?} escapes workspace {:?}", user_path, workspace),
            "Stay inside chat.workspace",
        ));
    }
    Ok(canonical)
}

pub fn execute_readonly_file_tool(workspace: &Path, name: &str, args: &Value) -> Result<String> {
    match name {
        READ_FILE => read_file(workspace, args),
        GREP => grep(workspace, args),
        GLOB => glob_files(workspace, args),
        other => Err(file_err(
            "file_tool",
            "FILE_TOOL_UNKNOWN",
            format!("unknown readonly file tool: {other}"),
            "Use read_file, grep, or glob",
        )),
    }
}

pub fn execute_mutating_file_tool(workspace: &Path, name: &str, args: &Value) -> Result<String> {
    match name {
        EDIT_FILE => edit_file(workspace, args),
        WRITE_FILE => write_file(workspace, args),
        other => Err(file_err(
            "file_tool",
            "FILE_TOOL_UNKNOWN",
            format!("unknown mutating file tool: {other}"),
            "Use edit_file or write_file",
        )),
    }
}

pub fn execute_file_tool(workspace: &Path, name: &str, args: &Value) -> Result<String> {
    if is_mutating_file_tool(name) {
        execute_mutating_file_tool(workspace, name, args)
    } else {
        execute_readonly_file_tool(workspace, name, args)
    }
}

fn read_file(workspace: &Path, args: &Value) -> Result<String> {
    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            file_err(
                READ_FILE,
                "FILE_MISSING_ARG",
                "read_file needs path",
                "Pass workspace-relative path",
            )
        })?;
    let start_line = args
        .get("start_line")
        .and_then(|v| v.as_u64())
        .unwrap_or(1)
        .max(1) as usize;
    let max_lines = args
        .get("max_lines")
        .and_then(|v| v.as_u64())
        .unwrap_or(READ_FILE_DEFAULT_MAX_LINES as u64)
        .clamp(1, 5_000) as usize;

    let resolved = resolve_workspace_path(workspace, path)?;
    if !resolved.is_file() {
        return Err(file_err(
            READ_FILE,
            "FILE_NOT_FOUND",
            format!("read_file: {:?} is not a file", path),
            "Use glob to find files",
        ));
    }
    let text = load_text_file(READ_FILE, &resolved)?;
    let content = text.content;
    let lines: Vec<&str> = content.lines().collect();
    let total = lines.len();
    let start_idx = start_line.saturating_sub(1).min(total);
    let end_idx = (start_idx + max_lines).min(total);
    let mut out = format!(
        "path: {} (lines {}-{} of {total}) [utf-8, {}]\n",
        display_relative(workspace, &resolved),
        start_idx + 1,
        end_idx,
        text.line_ending.label(),
    );
    for (i, line) in lines[start_idx..end_idx].iter().enumerate() {
        out.push_str(&format!("{}|{line}\n", start_idx + i + 1));
    }
    if end_idx < total {
        out.push_str(&format!(
            "\n[truncated — {total} total lines; use start_line to read more]\n"
        ));
    }
    Ok(truncate_chars(&out, READ_FILE_MAX_OUTPUT_CHARS))
}

fn grep(workspace: &Path, args: &Value) -> Result<String> {
    let pattern = args
        .get("pattern")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            file_err(GREP, "FILE_MISSING_ARG", "grep needs pattern", "Pass a regex pattern")
        })?;
    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or(".");
    let resolved = resolve_workspace_path(workspace, path)?;
    if !resolved.exists() {
        return Err(file_err(
            GREP,
            "FILE_NOT_FOUND",
            format!("grep: {:?} not found", path),
            "Use glob to locate search root",
        ));
    }

    let mut cmd = Command::new("rg");
    cmd.arg("--line-number")
        .arg("--no-heading")
        .arg("--color=never")
        .arg(pattern)
        .arg(&resolved);
    let output = match cmd.output() {
        Ok(o) => o,
        Err(_) => {
            let mut fallback = Command::new("grep");
            fallback
                .arg("-rn")
                .arg("--")
                .arg(pattern)
                .arg(&resolved);
            fallback
                .output()
                .map_err(|e| {
                    file_err(
                        GREP,
                        "FILE_IO_ERROR",
                        format!("grep spawn failed: {e}"),
                        "Ensure rg or grep is installed",
                    )
                })?
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut lines: Vec<String> = stdout
        .lines()
        .map(|l| l.replace(&format!("{}", resolved.display()), &display_relative(workspace, &resolved)))
        .collect();
    if lines.len() > GREP_MAX_MATCHES {
        let omitted = lines.len() - GREP_MAX_MATCHES;
        lines.truncate(GREP_MAX_MATCHES);
        lines.push(format!("[cap: {omitted} more matches omitted]"));
    }
    if lines.is_empty() && output.status.success() {
        return Ok(format!("grep: no matches for `{pattern}` under {path}"));
    }
    let body = lines.join("\n");
    Ok(truncate_chars(
        &format!("grep `{pattern}` ({path}):\n{body}"),
        GREP_MAX_OUTPUT_CHARS,
    ))
}

fn glob_files(workspace: &Path, args: &Value) -> Result<String> {
    let pattern = args
        .get("pattern")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            file_err(GLOB, "FILE_MISSING_ARG", "glob needs pattern", "Pass e.g. **/*.rs")
        })?;
    let base = args
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or(".");
    let base_path = resolve_workspace_path(workspace, base)?;
    if !base_path.is_dir() {
        return Err(file_err(
            GLOB,
            "FILE_NOT_FOUND",
            format!("glob: {:?} is not a directory", base),
            "Pass an existing directory path",
        ));
    }

    let re = glob_pattern_to_regex(pattern)?;
    let mut matches = Vec::new();
    collect_glob_matches(&base_path, &base_path, &re, &mut matches)?;
    matches.sort();
    matches.dedup();
    if matches.len() > GLOB_MAX_FILES {
        let omitted = matches.len() - GLOB_MAX_FILES;
        matches.truncate(GLOB_MAX_FILES);
        matches.push(format!("… and {omitted} more (cap {GLOB_MAX_FILES})"));
    }
    let body = if matches.is_empty() {
        format!("glob `{pattern}` under {base}: (no matches)")
    } else {
        format!(
            "glob `{pattern}` under {base} ({}):\n{}",
            matches.len(),
            matches.join("\n")
        )
    };
    Ok(truncate_chars(&body, GLOB_MAX_OUTPUT_CHARS))
}

fn collect_glob_matches(
    workspace_root: &Path,
    dir: &Path,
    re: &regex::Regex,
    out: &mut Vec<String>,
) -> Result<()> {
    if out.len() > GLOB_MAX_FILES.saturating_mul(2) {
        return Ok(());
    }
    let entries = std::fs::read_dir(dir).map_err(|e| {
        file_err(
            GLOB,
            "FILE_IO_ERROR",
            format!("glob read_dir failed: {e}"),
            "Check directory permissions",
        )
    })?;
    for entry in entries {
        let entry = entry.map_err(|e| {
            file_err(GLOB, "FILE_IO_ERROR", format!("glob entry: {e}"), "Retry glob")
        })?;
        let path = entry.path();
        let rel = path
            .strip_prefix(workspace_root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        if re.is_match(&rel) {
            out.push(rel);
        }
        if path.is_dir() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name == ".git" || name == "target" || name == "node_modules" {
                continue;
            }
            collect_glob_matches(workspace_root, &path, re, out)?;
        }
    }
    Ok(())
}

fn glob_pattern_to_regex(pattern: &str) -> Result<regex::Regex> {
    let mut re = String::from("^");
    let chars: Vec<char> = pattern.replace('\\', "/").chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '*' => {
                if i + 1 < chars.len() && chars[i + 1] == '*' {
                    re.push_str(".*");
                    i += 2;
                    if i < chars.len() && chars[i] == '/' {
                        i += 1;
                    }
                    continue;
                }
                re.push_str("[^/]*");
            }
            '?' => re.push('.'),
            '.' | '+' | '^' | '$' | '|' | '(' | ')' | '[' | ']' | '{' | '}' => {
                re.push('\\');
                re.push(chars[i]);
            }
            _ => re.push(chars[i]),
        }
        i += 1;
    }
    re.push('$');
    regex::Regex::new(&re).map_err(|e| {
        file_err(
            GLOB,
            "FILE_INVALID_GLOB",
            format!("invalid glob: {e}"),
            "Use **/*.ext or src/**/*.rs style patterns",
        )
    })
}

fn write_file(workspace: &Path, args: &Value) -> Result<String> {
    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            file_err(
                WRITE_FILE,
                "FILE_MISSING_ARG",
                "write_file needs path",
                "Pass workspace-relative path",
            )
        })?;
    let content = args
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            file_err(
                WRITE_FILE,
                "FILE_MISSING_ARG",
                "write_file needs content",
                "Pass file body as content",
            )
        })?;
    let create_only = args
        .get("create_only")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let resolved = resolve_workspace_path(workspace, path)?;
    if create_only && resolved.exists() {
        return Err(file_err(
            WRITE_FILE,
            "FILE_EXISTS",
            format!("write_file: {path} already exists (create_only=true)"),
            "Use edit_file or set create_only=false to overwrite",
        ));
    }
    let body = if resolved.is_file() {
        let existing = load_text_file(WRITE_FILE, &resolved)?;
        let normalized = normalize_line_endings_for_file(content, existing.line_ending);
        preserve_trailing_newline(existing.had_trailing_newline, &normalized)
    } else {
        preserve_trailing_newline(true, content)
    };
    atomic_write(&resolved, body.as_bytes())?;
    let rel = display_relative(workspace, &resolved);
    let lines = body.lines().count();
    let action = if create_only || !resolved.exists() {
        "created"
    } else {
        "overwrote"
    };
    Ok(format!(
        "write_file: {rel} ({action}, {lines} lines, {} bytes)",
        body.len()
    ))
}

fn edit_file(workspace: &Path, args: &Value) -> Result<String> {
    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            file_err(
                EDIT_FILE,
                "FILE_MISSING_ARG",
                "edit_file needs path",
                "Pass workspace-relative path",
            )
        })?;
    let old_str = args
        .get("old_string")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            file_err(
                EDIT_FILE,
                "FILE_MISSING_ARG",
                "edit_file needs old_string",
                "Copy exact text from read_file",
            )
        })?;
    let new_str = args
        .get("new_string")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            file_err(
                EDIT_FILE,
                "FILE_MISSING_ARG",
                "edit_file needs new_string",
                "Provide replacement text",
            )
        })?;

    let replace_all = args
        .get("replace_all")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let resolved = resolve_workspace_path(workspace, path)?;
    if !resolved.is_file() {
        return Err(file_err(
            EDIT_FILE,
            "FILE_NOT_FOUND",
            format!("edit_file: {:?} is not a file", path),
            "Use read_file/glob to confirm path",
        ));
    }
    let text = load_text_file(EDIT_FILE, &resolved)?;
    let candidates = old_string_candidates(old_str, &text);
    let (matched, mode, count) = match_old_string(&text.content, &candidates, replace_all)?;

    let updated = match mode {
        MatchMode::NotFound => {
            return Err(file_err(
                EDIT_FILE,
                "FILE_NOT_FOUND",
                format!("edit_file: old_string not found in {path}"),
                "Use read_file to copy exact old_string (LF/CRLF normalized automatically)",
            ));
        }
        MatchMode::Ambiguous => {
            return Err(file_err(
                EDIT_FILE,
                "FILE_AMBIGUOUS_EDIT",
                format!("edit_file: old_string matches {count} times in {path} — must be unique"),
                "Include more context lines in old_string, or set replace_all=true",
            ));
        }
        MatchMode::Unique | MatchMode::All => {
            let new_body = normalize_line_endings_for_file(new_str, text.line_ending);
            let updated = apply_replacement(&text.content, &matched, &new_body, mode);
            preserve_file_eof(&text, updated)
        }
    };
    atomic_write(&resolved, updated.as_bytes())?;

    let old_lines = old_str.lines().count();
    let new_lines = new_str.lines().count();
    let delta = (new_lines as i32 - old_lines as i32)
        * if mode == MatchMode::All { count as i32 } else { 1 };
    let rel = display_relative(workspace, &resolved);
    let suffix = if mode == MatchMode::All {
        format!(", {count} replacements")
    } else {
        String::new()
    };
    Ok(format!("edit_file: {rel} ({delta:+} lines{suffix})"))
}

/// Write bytes atomically via temp file + rename in the target directory.
pub fn atomic_write(path: &Path, content: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            file_err(
                WRITE_FILE,
                "FILE_IO_ERROR",
                format!("mkdir failed: {e}"),
                "Check parent path permissions",
            )
        })?;
    }
    let tmp = path.with_extension(format!(
        "tmp.{}",
        uuid::Uuid::new_v4().as_simple()
    ));
    std::fs::write(&tmp, content).map_err(|e| {
        file_err(
            WRITE_FILE,
            "FILE_IO_ERROR",
            format!("write temp failed: {e}"),
            "Check disk space and permissions",
        )
    })?;
    std::fs::rename(&tmp, path).map_err(|e| {
        file_err(
            WRITE_FILE,
            "FILE_IO_ERROR",
            format!("rename failed: {e}"),
            "Retry write_file",
        )
    })?;
    Ok(())
}

pub fn format_edit_summary(workspace: &Path, path: &str, tool_name: &str, output: &str) -> String {
    let _ = workspace;
    if let Some(rest) = output.strip_prefix("edit_file: ") {
        return rest.to_string();
    }
    if let Some(rest) = output.strip_prefix("write_file: ") {
        return rest.to_string();
    }
    format!("{tool_name} {path}")
}

pub(crate) fn display_relative(workspace: &Path, path: &Path) -> String {
    path.strip_prefix(workspace.canonicalize().unwrap_or_else(|_| workspace.to_path_buf()))
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use tempfile::TempDir;

    fn workspace_with_files() -> (TempDir, PathBuf) {
        let dir = TempDir::new().unwrap();
        let root = dir.path().to_path_buf();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/foo.rs"), "fn main() {\n    println!(\"hi\");\n}\n").unwrap();
        fs::write(root.join("README.md"), "# test\n").unwrap();
        fs::create_dir_all(root.join("nested")).unwrap();
        fs::write(root.join("nested/bar.txt"), "bar\n").unwrap();
        (dir, root)
    }

    #[test]
    fn sandbox_rejects_parent_escape() {
        let (_dir, root) = workspace_with_files();
        let err = resolve_workspace_path(&root, "../outside").unwrap_err();
        assert!(err.to_string().contains(".."));
    }

    #[test]
    fn sandbox_allows_relative_file() {
        let (_dir, root) = workspace_with_files();
        let p = resolve_workspace_path(&root, "src/foo.rs").unwrap();
        assert!(p.ends_with("foo.rs"));
    }

    #[test]
    fn read_file_line_range() {
        let (_dir, root) = workspace_with_files();
        let out = read_file(&root, &json!({"path": "src/foo.rs", "start_line": 2, "max_lines": 1})).unwrap();
        assert!(out.contains("2|"));
        assert!(out.contains("println"));
    }

    #[test]
    fn edit_file_atomic_unique_match() {
        let (_dir, root) = workspace_with_files();
        edit_file(
            &root,
            &json!({
                "path": "src/foo.rs",
                "old_string": "println!(\"hi\")",
                "new_string": "println!(\"bye\")"
            }),
        )
        .unwrap();
        let content = fs::read_to_string(root.join("src/foo.rs")).unwrap();
        assert!(content.contains("bye"));
    }

    #[test]
    fn edit_file_rejects_ambiguous_old_string() {
        let (_dir, root) = workspace_with_files();
        fs::write(root.join("dup.txt"), "x\nx\n").unwrap();
        let err = edit_file(
            &root,
            &json!({
                "path": "dup.txt",
                "old_string": "x",
                "new_string": "y"
            }),
        )
        .unwrap_err();
        assert!(err.to_string().contains("unique"));
    }

    #[test]
    fn grep_caps_output() {
        let (_dir, root) = workspace_with_files();
        for i in 0..250 {
            fs::write(root.join(format!("many_{i}.txt")), "needle\n").unwrap();
        }
        let out = grep(
            &root,
            &json!({"pattern": "needle", "path": "."}),
        )
        .unwrap();
        assert!(out.contains("[cap:"));
    }

    #[test]
    fn glob_finds_rust_files() {
        let (_dir, root) = workspace_with_files();
        let out = glob_files(&root, &json!({"pattern": "**/*.rs", "path": "."})).unwrap();
        assert!(out.contains("src/foo.rs"));
    }

    #[test]
    fn write_file_creates_new() {
        let (_dir, root) = workspace_with_files();
        write_file(
            &root,
            &json!({"path": "new.txt", "content": "hello\n"}),
        )
        .unwrap();
        assert_eq!(fs::read_to_string(root.join("new.txt")).unwrap(), "hello\n");
    }

    #[test]
    fn edit_file_matches_crlf_with_lf_old_string() {
        let (_dir, root) = workspace_with_files();
        fs::write(root.join("crlf.txt"), "foo\r\nbar\r\n").unwrap();
        edit_file(
            &root,
            &json!({
                "path": "crlf.txt",
                "old_string": "foo\nbar\n",
                "new_string": "baz\n"
            }),
        )
        .unwrap();
        assert_eq!(fs::read(root.join("crlf.txt")).unwrap(), b"baz\r\n");
    }

    #[test]
    fn edit_file_replace_all() {
        let (_dir, root) = workspace_with_files();
        fs::write(root.join("many.txt"), "x\nx\nx\n").unwrap();
        edit_file(
            &root,
            &json!({
                "path": "many.txt",
                "old_string": "x",
                "new_string": "y",
                "replace_all": true
            }),
        )
        .unwrap();
        assert_eq!(fs::read_to_string(root.join("many.txt")).unwrap(), "y\ny\ny\n");
    }

    #[test]
    fn write_file_create_only_rejects_existing() {
        let (_dir, root) = workspace_with_files();
        let err = write_file(
            &root,
            &json!({"path": "src/foo.rs", "content": "x", "create_only": true}),
        )
        .unwrap_err();
        assert!(err.to_string().contains("FILE_EXISTS"));
    }

    #[test]
    fn write_file_preserves_crlf_on_overwrite() {
        let (_dir, root) = workspace_with_files();
        fs::write(root.join("win.txt"), "a\r\nb\r\n").unwrap();
        write_file(
            &root,
            &json!({"path": "win.txt", "content": "c\nd\n"}),
        )
        .unwrap();
        assert_eq!(fs::read(root.join("win.txt")).unwrap(), b"c\r\nd\r\n");
    }

    #[test]
    fn edit_file_rejects_binary() {
        let (_dir, root) = workspace_with_files();
        fs::write(root.join("bin.dat"), b"ok\0bad").unwrap();
        let err = edit_file(
            &root,
            &json!({
                "path": "bin.dat",
                "old_string": "ok",
                "new_string": "no"
            }),
        )
        .unwrap_err();
        assert!(err.to_string().contains("FILE_BINARY"));
    }
}
