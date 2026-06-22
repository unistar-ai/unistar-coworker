use regex::Regex;
use std::sync::LazyLock;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedPrLine {
    pub number: u32,
    pub title: String,
    pub author: String,
    pub ci: String,
    pub review: String,
    pub is_draft: bool,
}

#[derive(Debug, Clone)]
pub struct ParsedRunLine {
    pub run_id: i64,
    pub workflow: String,
    pub conclusion: String,
}

static PR_LINE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^#(\d+)\s+(.+?)\s+@(\S+)\s+CI:(\S+)\s+review:(\S+)(.*)$").unwrap()
});

static SIMPLE_PR_LINE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^#(\d+)\s+(.+?)\s+@(\S+)\s+(?:updated|merged):").unwrap());

static RUN_LINE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(\d+)\s+(.+?)\s+(\S+)(?:\s+(\S+))?\s*$").unwrap());

static BRANCH_HEADER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^branch:\s+(\S+)").unwrap());

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedBranchRun {
    pub run_id: i64,
    pub workflow: String,
    pub conclusion: String,
    /// Wall-clock duration when MCP includes it (e.g. `4m12s`); `-` when pending.
    pub duration: Option<String>,
}

pub fn parse_branch_runs(text: &str) -> (Option<String>, Vec<ParsedBranchRun>) {
    let mut branch = None;
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('(') {
            continue;
        }
        if let Some(caps) = BRANCH_HEADER.captures(line) {
            branch = Some(caps[1].to_string());
            continue;
        }
        if line.contains(" run(s) for ") {
            continue;
        }
        if let Some(caps) = RUN_LINE.captures(line) {
            if let Ok(run_id) = caps[1].parse::<i64>() {
                out.push(ParsedBranchRun {
                    run_id,
                    workflow: caps[2].trim().to_string(),
                    conclusion: caps[3].trim().to_string(),
                    duration: caps.get(4).map(|m| m.as_str().to_string()),
                });
            }
        }
    }
    (branch, out)
}

pub fn run_conclusion_is_failure(conclusion: &str) -> bool {
    matches!(
        conclusion.to_ascii_lowercase().as_str(),
        "failure" | "timed_out" | "startup_failure" | "action_required" | "cancelled"
    )
}

/// Parse MCP compact duration strings (`45s`, `4m12s`, `2h5m`). Returns `None` for `-` or empty.
pub fn parse_compact_duration(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.is_empty() || s == "-" {
        return None;
    }
    let mut total = 0u64;
    let mut num = String::new();
    for ch in s.chars() {
        if ch.is_ascii_digit() {
            num.push(ch);
            continue;
        }
        let n: u64 = num.parse().ok()?;
        num.clear();
        match ch {
            'h' => total += n * 3600,
            'm' => total += n * 60,
            's' => total += n,
            _ => return None,
        }
    }
    if !num.is_empty() {
        return None;
    }
    Some(total)
}

/// Format seconds into MCP-style compact duration (`4m12s`, `2h5m`, `45s`).
pub fn format_compact_duration(secs: u64) -> String {
    if secs >= 3600 {
        let h = secs / 3600;
        let m = (secs % 3600) / 60;
        if m == 0 {
            format!("{h}h")
        } else {
            format!("{h}h{m}m")
        }
    } else if secs >= 60 {
        let m = secs / 60;
        let s = secs % 60;
        if s == 0 {
            format!("{m}m")
        } else {
            format!("{m}m{s}s")
        }
    } else {
        format!("{secs}s")
    }
}

pub fn parse_pr_line(line: &str) -> Option<ParsedPrLine> {
    let line = line.trim();
    if line.is_empty()
        || line.starts_with("open PR")
        || line.starts_with("stale open PR")
        || line.starts_with("merged PR")
        || line.contains(" waiting for review in ")
        || line.starts_with('(')
    {
        return None;
    }
    if let Some(caps) = SIMPLE_PR_LINE.captures(line) {
        return Some(ParsedPrLine {
            number: caps[1].parse().ok()?,
            title: caps[2].trim().to_string(),
            author: caps[3].to_string(),
            ci: "unknown".into(),
            review: "none".into(),
            is_draft: false,
        });
    }
    let caps = PR_LINE.captures(line)?;
    let tail = caps.get(6).map(|m| m.as_str()).unwrap_or("");
    Some(ParsedPrLine {
        number: caps[1].parse().ok()?,
        title: caps[2].trim().to_string(),
        author: caps[3].to_string(),
        review: caps[5].to_string(),
        ci: caps[4].to_string(),
        is_draft: tail.contains("[draft]") || line.contains("[draft]"),
    })
}

pub fn parse_failing_runs(text: &str) -> Vec<ParsedRunLine> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty()
            || line.starts_with("No failing")
            || line.starts_with("CI_KIND:")
            || line.contains("failing run(s)")
            || line.starts_with('(')
        {
            continue;
        }
        if let Some(caps) = RUN_LINE.captures(line) {
            if let Ok(run_id) = caps[1].parse::<i64>() {
                out.push(ParsedRunLine {
                    run_id,
                    workflow: caps[2].trim().to_string(),
                    conclusion: caps[3].to_string(),
                });
            }
        }
    }
    out
}

/// First-line `CI_KIND` from `ci_analyze_pr_failures` (actions_only / external_only / mixed / …).
pub fn extract_ci_kind(text: &str) -> Option<&str> {
    text.lines()
        .find_map(|l| l.strip_prefix("CI_KIND:").map(str::trim))
}

/// Reuse the failing-runs block from `pr_get_overview` to skip a separate analyze call.
pub fn extract_failing_runs_from_overview(overview: &str) -> Option<String> {
    if overview.contains("Failing CI runs: none") {
        return Some("No failing GitHub Actions runs (from pr_get_overview).".to_string());
    }
    for (i, line) in overview.lines().enumerate() {
        if line.contains(" failing run(s) for PR #") {
            return Some(overview.lines().skip(i).collect::<Vec<_>>().join("\n"));
        }
    }
    None
}

pub fn ci_is_failing(ci: &str) -> bool {
    let c = ci.to_ascii_lowercase();
    c.starts_with("failing") || c.contains("fail")
}

pub fn ci_is_passing(ci: &str) -> bool {
    ci.to_ascii_lowercase().starts_with("passing")
}

pub fn needs_review(review: &str) -> bool {
    matches!(
        review.to_ascii_lowercase().as_str(),
        "review-required" | "changes-requested"
    )
}

pub fn is_review_required(review: &str) -> bool {
    review.eq_ignore_ascii_case("review-required")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedIssueLine {
    pub number: u32,
    pub title: String,
    pub author: String,
    pub labels: String,
    pub updated: String,
}

static ISSUE_LINE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^#(\d+)\s+(.+?)\s+@(\S+)\s+labels:(\S+)\s+updated:(\S+)$").unwrap()
});

pub fn parse_issue_line(line: &str) -> Option<ParsedIssueLine> {
    let line = line.trim();
    if !line.starts_with('#') {
        return None;
    }
    let caps = ISSUE_LINE.captures(line)?;
    Some(ParsedIssueLine {
        number: caps[1].parse().ok()?,
        title: caps[2].trim().to_string(),
        author: caps[3].to_string(),
        labels: caps[4].to_string(),
        updated: caps[5].to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_failing_runs_from_overview_section() {
        let overview =
            "PR #1 title\nFiles: 2\n2 failing run(s) for PR #1 @abc1234:\n123  wf  failure\n";
        let section = extract_failing_runs_from_overview(overview).unwrap();
        let runs = parse_failing_runs(&section);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].run_id, 123);
    }

    #[test]
    fn parse_pr() {
        let p = parse_pr_line("#42  fix bug  @alice  CI:failing(1)  review:none").unwrap();
        assert_eq!(p.number, 42);
        assert!(ci_is_failing(&p.ci));
        assert!(!ci_is_passing(&p.ci));
    }

    #[test]
    fn review_radar_filter() {
        let p = parse_pr_line("#1  feat  @bob  CI:passing  review:review-required").unwrap();
        assert!(ci_is_passing(&p.ci));
        assert!(is_review_required(&p.review));
        let pending = parse_pr_line("#2  feat  @bob  CI:pending  review:review-required").unwrap();
        assert!(!ci_is_passing(&pending.ci));
    }

    #[test]
    fn parse_runs() {
        let text = "2 failing run(s) for PR #1 @abc:\n12345  CI  failure\n";
        let runs = parse_failing_runs(text);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].run_id, 12345);
    }

    #[test]
    fn parse_runs_skips_ci_kind() {
        let text = "CI_KIND: actions_only\n2 failing run(s) for PR #1 @abc:\n12345  CI  failure\n";
        let runs = parse_failing_runs(text);
        assert_eq!(runs.len(), 1);
        assert_eq!(extract_ci_kind(text), Some("actions_only"));
    }

    #[test]
    fn parse_issue() {
        let i =
            parse_issue_line("#99  bug report  @bob  labels:bug,p1  updated:2026-06-12").unwrap();
        assert_eq!(i.number, 99);
        assert_eq!(i.author, "bob");
    }

    #[test]
    fn parse_compact_duration_variants() {
        assert_eq!(parse_compact_duration("45s"), Some(45));
        assert_eq!(parse_compact_duration("4m12s"), Some(252));
        assert_eq!(parse_compact_duration("2h5m"), Some(7500));
        assert_eq!(parse_compact_duration("-"), None);
        assert_eq!(parse_compact_duration(""), None);
    }

    #[test]
    fn format_compact_duration_roundtrip() {
        assert_eq!(format_compact_duration(45), "45s");
        assert_eq!(format_compact_duration(252), "4m12s");
        assert_eq!(format_compact_duration(7500), "2h5m");
    }

    #[test]
    fn parse_branch_run_lines() {
        let text = "branch: main\n3 run(s) for org/repo:\n100  CI  failure  4m12s\n101  Build  failure  2m0s\n102  Lint  success  1m30s\n";
        let (branch, runs) = super::parse_branch_runs(text);
        assert_eq!(branch.as_deref(), Some("main"));
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[0].duration.as_deref(), Some("4m12s"));
    }
}
