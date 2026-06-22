//! Normalize command output and terminal control sequences for safe TUI rendering.

/// Apply `\r` as "return to start of line" (curl/wget progress bars overwrite the same row).
pub fn apply_carriage_returns(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\r' => {
                if let Some(pos) = result.rfind('\n') {
                    result.truncate(pos + 1);
                } else {
                    result.clear();
                }
            }
            c => result.push(c),
        }
    }
    result
}

/// Strip OSC/CSI sequences so display-width math and ratatui layout stay aligned.
pub fn strip_terminal_escapes(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b {
            i += 1;
            if i < bytes.len() && bytes[i] == b']' {
                while i < bytes.len() {
                    if bytes[i] == 0x1b && bytes.get(i + 1) == Some(&b'\\') {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
            } else if i < bytes.len() && bytes[i] == b'[' {
                i += 1;
                while i < bytes.len() && !bytes[i].is_ascii_alphabetic() {
                    i += 1;
                }
                if i < bytes.len() {
                    i += 1;
                }
            } else {
                i += 1;
            }
        } else {
            let ch = s[i..].chars().next().unwrap_or('\0');
            if ch == '\0' {
                break;
            }
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

/// Sanitize captured shell output before storing or rendering in the TUI.
pub fn sanitize_terminal_output(s: &str) -> String {
    strip_terminal_escapes(&apply_carriage_returns(s))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_carriage_returns_collapses_curl_progress() {
        let raw = "  % Total    % Received % Xferd  Average Speed\r  0     0    0     0    0     0\r100  116k  100  116k    0     0   101k      0  0:00:01  0:00:01 --:--:--  101k\n";
        let cleaned = sanitize_terminal_output(raw);
        assert!(!cleaned.contains('\r'));
        assert!(cleaned.contains("100  116k"));
        assert!(!cleaned.contains("% Total"));
    }

    #[test]
    fn apply_carriage_returns_respects_newlines() {
        let raw = "line one\roverwrite\nline two\rfix";
        let cleaned = apply_carriage_returns(raw);
        assert_eq!(cleaned, "overwrite\nfix");
    }
}
