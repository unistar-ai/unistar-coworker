use std::io::Write;
use std::process::{Command, Stdio};

/// Copy plain text to the system clipboard. Returns false when no backend is available.
pub fn copy_text(text: &str) -> bool {
    if text.is_empty() {
        return false;
    }
    #[cfg(target_os = "macos")]
    {
        copy_via_stdin(&["pbcopy"], text)
    }
    #[cfg(target_os = "linux")]
    {
        if copy_via_stdin(&["wl-copy"], text) {
            true
        } else {
            copy_via_stdin(&["xclip", "-selection", "clipboard"], text)
        }
    }
    #[cfg(target_os = "windows")]
    {
        copy_via_stdin(&["clip"], text)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        let _ = text;
        false
    }
}

fn copy_via_stdin(cmd: &[&str], text: &str) -> bool {
    let Some((program, args)) = cmd.split_first() else {
        return false;
    };
    Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .spawn()
        .ok()
        .and_then(|mut child| {
            child
                .stdin
                .take()
                .and_then(|mut stdin| stdin.write_all(text.as_bytes()).ok())
                .and_then(|_| child.wait().ok())
                .map(|status| status.success())
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_text_is_not_copied() {
        assert!(!copy_text(""));
    }
}
