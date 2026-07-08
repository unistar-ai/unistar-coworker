/// Parse paginated output from `ci_get_failed_logs` (or treat legacy blobs as a single page).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogPage {
    pub body: String,
    pub has_more: bool,
    pub next_offset_lines: u32,
}

pub fn parse_log_page(response: &str) -> LogPage {
    let (header, body) = match response.split_once("\n\n") {
        Some((h, b)) => (h, b.trim()),
        None => ("", response.trim()),
    };

    let has_more = parse_bool_field(header, "has_more");
    let next_offset = parse_u32_field(header, "next_offset_lines");

    if header.contains("next_offset_lines:") {
        return LogPage {
            body: body.to_string(),
            has_more,
            next_offset_lines: next_offset,
        };
    }

    // Legacy single-chunk response (no pagination header).
    LogPage {
        body: response.trim().to_string(),
        has_more: false,
        next_offset_lines: 0,
    }
}

fn parse_bool_field(header: &str, key: &str) -> bool {
    let needle = format!("{key}:");
    header
        .split(',')
        .chain(header.split('('))
        .find_map(|part| {
            let part = part.trim();
            part.strip_prefix(&needle)
                .map(|v| v.trim().eq_ignore_ascii_case("true"))
        })
        .unwrap_or(false)
}

fn parse_u32_field(header: &str, key: &str) -> u32 {
    let needle = format!("{key}:");
    header
        .split([',', ')'])
        .find_map(|part| {
            let part = part.trim();
            part.strip_prefix(&needle)
                .and_then(|v| v.trim().parse().ok())
        })
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_paged_header() {
        let raw = "Run 1 — error lines 81-160 of 450 (page 2/6, has_more: true, next_offset_lines: 160)\n\nline81\nline82";
        let p = parse_log_page(raw);
        assert_eq!(p.body, "line81\nline82");
        assert!(p.has_more);
        assert_eq!(p.next_offset_lines, 160);
    }

    #[test]
    fn parse_legacy_blob() {
        let raw = "Run 1 — 3 error line(s):\n\npanic!";
        let p = parse_log_page(raw);
        assert!(!p.has_more);
        assert!(p.body.contains("panic!"));
    }
}
