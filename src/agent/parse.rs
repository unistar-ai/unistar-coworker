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

static RUN_LINE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(\d+)\s+(.+?)\s+(\S+)\s*$").unwrap());

pub fn parse_pr_line(line: &str) -> Option<ParsedPrLine> {
    let line = line.trim();
    if line.is_empty() || line.starts_with("open PR") || line.starts_with('(') {
        return None;
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

pub fn ci_is_failing(ci: &str) -> bool {
    let c = ci.to_ascii_lowercase();
    c.starts_with("failing") || c.contains("fail")
}

pub fn needs_review(review: &str) -> bool {
    matches!(
        review.to_ascii_lowercase().as_str(),
        "review-required" | "changes-requested"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_pr() {
        let p = parse_pr_line("#42  fix bug  @alice  CI:failing(1)  review:none").unwrap();
        assert_eq!(p.number, 42);
        assert!(ci_is_failing(&p.ci));
    }

    #[test]
    fn parse_runs() {
        let text = "2 failing run(s) for PR #1 @abc:\n12345  CI  failure\n";
        let runs = parse_failing_runs(text);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].run_id, 12345);
    }
}
