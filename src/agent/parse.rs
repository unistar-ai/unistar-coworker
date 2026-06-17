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
    LazyLock::new(|| Regex::new(r"^(\d+)\s+(.+?)\s+(\S+)\s*$").unwrap());

static BRANCH_HEADER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^branch:\s+(\S+)").unwrap());

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedBranchRun {
    pub run_id: i64,
    pub workflow: String,
    pub conclusion: String,
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

pub fn leading_failure_streak(runs: &[ParsedBranchRun]) -> u32 {
    let mut count = 0u32;
    for run in runs {
        let c = run.conclusion.to_ascii_lowercase();
        if c.is_empty() || matches!(c.as_str(), "in_progress" | "queued" | "waiting" | "pending") {
            continue;
        }
        if run_conclusion_is_failure(&c) {
            count += 1;
        } else {
            break;
        }
    }
    count
}

pub fn github_actions_run_url(repo: &str, run_id: i64) -> String {
    format!("https://github.com/{repo}/actions/runs/{run_id}")
}

pub fn github_pr_url(repo: &str, number: u32) -> String {
    format!("https://github.com/{repo}/pull/{number}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MyPrCategory {
    Draft,
    CiFailing,
    WaitingReview,
    Ready,
}

pub fn categorize_my_pr(pr: &ParsedPrLine) -> MyPrCategory {
    if pr.is_draft {
        return MyPrCategory::Draft;
    }
    if ci_is_failing(&pr.ci) {
        return MyPrCategory::CiFailing;
    }
    if needs_review(&pr.review) && pr.review != "approved" {
        return MyPrCategory::WaitingReview;
    }
    if ci_is_passing(&pr.ci) && pr.review == "approved" {
        return MyPrCategory::Ready;
    }
    MyPrCategory::WaitingReview
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

pub fn merge_blockers_summary(text: &str) -> String {
    let mut out = Vec::new();
    let mut in_blockers = false;
    for line in text.lines() {
        if line.starts_with("Blockers:") {
            in_blockers = true;
            if line.contains("(none)") {
                return String::new();
            }
            continue;
        }
        if in_blockers {
            if let Some(rest) = line.strip_prefix("- ") {
                out.push(rest.to_string());
            }
        } else if line.starts_with("Mergeable:") {
            out.push(line.to_string());
        }
    }
    out.join("; ")
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
    fn merge_blockers_summary_parses_list() {
        let text =
            "PR #1 t\nMergeable: no\nBlockers:\n- review required\n- CI failing: lint (failure)";
        let s = merge_blockers_summary(text);
        assert!(s.contains("review required"));
        assert!(s.contains("CI failing"));
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
    fn categorize_my_pr_states() {
        let failing = parse_pr_line("#1  a  @me  CI:failing(1)  review:none").unwrap();
        assert_eq!(categorize_my_pr(&failing), MyPrCategory::CiFailing);
        let ready = parse_pr_line("#2  b  @me  CI:passing  review:approved").unwrap();
        assert_eq!(categorize_my_pr(&ready), MyPrCategory::Ready);
    }

    #[test]
    fn parse_runs() {
        let text = "2 failing run(s) for PR #1 @abc:\n12345  CI  failure\n";
        let runs = parse_failing_runs(text);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].run_id, 12345);
    }

    #[test]
    fn parse_issue() {
        let i =
            parse_issue_line("#99  bug report  @bob  labels:bug,p1  updated:2026-06-12").unwrap();
        assert_eq!(i.number, 99);
        assert_eq!(i.author, "bob");
    }

    #[test]
    fn parse_branch_runs_and_streak() {
        let text = "branch: main\n3 run(s) for org/repo:\n100  CI  failure\n101  Build  failure\n102  Lint  success\n";
        let (branch, runs) = parse_branch_runs(text);
        assert_eq!(branch.as_deref(), Some("main"));
        assert_eq!(runs.len(), 3);
        assert_eq!(leading_failure_streak(&runs), 2);
    }
}
