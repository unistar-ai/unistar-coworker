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
    let issues = review
        .critical_issues
        .iter()
        .filter_map(format_issue_line)
        .collect::<Vec<_>>()
        .join("; ");
    let issues = if issues.is_empty() {
        format!("reason={}", review.reason_code)
    } else {
        issues
    };
    format!("Chat: {tool_name} — LLM safety review REJECT ({issues})")
}

fn format_issue_line(issue: &BashCriticalIssue) -> Option<String> {
    let desc = issue.description.trim();
    if desc.is_empty() {
        let rt = issue.risk_type.trim();
        if rt.is_empty() {
            None
        } else {
            Some(rt.to_string())
        }
    } else {
        Some(desc.to_string())
    }
}
