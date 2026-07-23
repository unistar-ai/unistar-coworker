//! LLM safety review gate — approve, reject → human approval fallback.

use crate::agent::bash_tool::BASH_RUN_TOOL;
use crate::agent::bash_tool::{BashCommandReview, BashCriticalIssue};
use crate::agent::file_tools::{EDIT_FILE, WRITE_FILE};
use crate::agent::python_tool::PYTHON_RUN_TOOL;
use crate::store::ApprovalKind;

#[derive(Debug, Clone)]
pub enum ReviewGateOutcome {
    Executed(String),
    LlmRejected(BashCommandReview),
}

pub fn is_review_gated_tool(name: &str) -> bool {
    matches!(
        name,
        BASH_RUN_TOOL | PYTHON_RUN_TOOL | EDIT_FILE | WRITE_FILE
    )
}

pub fn approval_kind_for_review_gated_tool(tool_name: &str) -> Option<ApprovalKind> {
    match tool_name {
        BASH_RUN_TOOL => Some(ApprovalKind::BashRun),
        PYTHON_RUN_TOOL => Some(ApprovalKind::PythonRun),
        WRITE_FILE => Some(ApprovalKind::WriteFile),
        EDIT_FILE => Some(ApprovalKind::EditFile),
        _ => None,
    }
}

pub fn format_review_rejection_description(tool_name: &str, review: &BashCommandReview) -> String {
    let mut issue_parts: Vec<String> = review
        .critical_issues
        .iter()
        .filter_map(format_issue_line)
        .collect();

    if issue_parts.is_empty() {
        for issue in &review.critical_issues {
            if let Some(line) = format_issue_fallback(issue) {
                issue_parts.push(line);
            }
        }
    }

    for suggestion in &review.suggestions {
        let t = suggestion.trim();
        if t.is_empty() {
            continue;
        }
        if !issue_parts.iter().any(|p| p == t) {
            issue_parts.push(t.to_string());
        }
    }

    let issues = if issue_parts.is_empty() {
        rejection_summary_fallback(review)
    } else {
        issue_parts.join("; ")
    };
    format!("Chat: {tool_name} — LLM safety review REJECT ({issues})")
}

fn format_issue_line(issue: &BashCriticalIssue) -> Option<String> {
    let desc = issue.description.trim();
    if desc.is_empty() {
        return None;
    }
    let rt = issue.risk_type.trim();
    if rt.is_empty() {
        Some(desc.to_string())
    } else if desc.to_ascii_lowercase().contains(&rt.to_ascii_lowercase()) {
        Some(desc.to_string())
    } else {
        Some(format!("{rt}: {desc}"))
    }
}

fn format_issue_fallback(issue: &BashCriticalIssue) -> Option<String> {
    let rt = issue.risk_type.trim();
    let snip = issue.code_snippet.trim();
    if !rt.is_empty() && !snip.is_empty() {
        return Some(format!("{rt}: {snip}"));
    }
    if !rt.is_empty() {
        return Some(rt.to_string());
    }
    if !snip.is_empty() {
        return Some(snip.to_string());
    }
    None
}

fn rejection_summary_fallback(review: &BashCommandReview) -> String {
    let rc = review.reason_code.trim();
    if rc.eq_ignore_ascii_case("RISK_FOUND") {
        return "Automated safety check flagged risks — review the command above before approving."
            .to_string();
    }
    if rc.eq_ignore_ascii_case("SUCCESS") {
        return "Review returned REJECT — verify the payload before approving.".to_string();
    }
    if rc.is_empty() || looks_like_shell_payload(rc) {
        return "Automated safety check rejected this action — see the command/payload above."
            .to_string();
    }
    format!("Review note: {rc}")
}

fn looks_like_shell_payload(s: &str) -> bool {
    let s = s.trim();
    if s.len() > 72 {
        return true;
    }
    let lower = s.to_ascii_lowercase();
    lower.starts_with("gh ")
        || lower.starts_with("export ")
        || lower.starts_with("curl ")
        || lower.starts_with("sudo ")
        || lower.contains(" && ")
        || lower.contains('|')
        || lower.contains("${")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejection_description_uses_issue_text() {
        let review = BashCommandReview {
            verdict: "REJECT".into(),
            reason_code: "RISK_FOUND".into(),
            critical_issues: vec![BashCriticalIssue {
                line_number: 1,
                code_snippet: "rm -rf".into(),
                risk_type: "HIGH_RISK_COMMAND".into(),
                description: "Destructive delete".into(),
            }],
            suggestions: vec![],
        };
        let d = format_review_rejection_description("bash_run", &review);
        assert!(d.contains("Destructive delete"));
        assert!(!d.contains("reason="));
    }

    #[test]
    fn rejection_description_avoids_echoing_command_as_reason() {
        let review = BashCommandReview {
            verdict: "REJECT".into(),
            reason_code: "gh api repos/foo -q '.name'".into(),
            critical_issues: vec![],
            suggestions: vec![],
        };
        let d = format_review_rejection_description("bash_run", &review);
        assert!(!d.contains("reason=gh"));
        assert!(d.contains("see the command"));
    }
}
