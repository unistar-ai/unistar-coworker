//! Format approval tool payloads for display (TUI modal, aligned with Web `buildApprovalPayload`).

use serde_json::Value;

use crate::store::{Approval, ApprovalKind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalPayloadSection {
    pub label: String,
    pub text: String,
}

/// Resolve serialized tool arguments from dialog state or an approval store row.
pub fn resolve_approval_tool_args(
    tool_args_json: Option<&str>,
    approval: Option<&Approval>,
) -> Option<String> {
    if let Some(s) = tool_args_json {
        if !s.trim().is_empty() {
            return Some(s.to_string());
        }
    }
    approval.and_then(synthesize_tool_args_from_approval)
}

/// Build labeled payload sections for UI display.
pub fn build_approval_payload_sections(
    tool_name: &str,
    tool_args_json: Option<&str>,
) -> Vec<ApprovalPayloadSection> {
    build_approval_payload_sections_with_approval(tool_name, tool_args_json, None)
}

pub fn build_approval_payload_sections_with_approval(
    tool_name: &str,
    tool_args_json: Option<&str>,
    approval: Option<&Approval>,
) -> Vec<ApprovalPayloadSection> {
    let name = tool_name;

    if let Some(a) = approval {
        if name == "pr_post_comment" {
            if let Some(body) = &a.comment_body {
                if !body.trim().is_empty() {
                    return vec![section("Comment body", body)];
                }
            }
        }
        if name == "issue_add_label" {
            let mut parts = Vec::new();
            parts.push(format!("repo: {}", a.repo));
            if let Some(issue) = a.issue_number {
                parts.push(format!("issue: #{issue}"));
            }
            if let Some(label) = &a.label {
                parts.push(format!("label: {label}"));
            }
            if !parts.is_empty() {
                return vec![section("Details", &parts.join("\n"))];
            }
        }
        if name == "ci_rerun_workflow" {
            if let Some(run_id) = a.run_id {
                return vec![section(
                    "Details",
                    &format!("repo: {}\nrun_id: {run_id}", a.repo),
                )];
            }
        }
        if name == "pr_create_backport" {
            return vec![section(
                "Details",
                &format!(
                    "repo: {}\nPR: #{}\ntarget: {}",
                    a.repo,
                    a.pr_number
                        .map(|n| n.to_string())
                        .unwrap_or_else(|| "?".into()),
                    a.target_branch.as_deref().unwrap_or("?")
                ),
            )];
        }
    }

    let Some(raw_json) = resolve_approval_tool_args(tool_args_json, approval) else {
        return Vec::new();
    };

    if name == "pr_post_comment" && serde_json::from_str::<Value>(&raw_json).is_err() {
        return vec![section("Comment body", &raw_json)];
    }

    let Some(info) = parse_approval_args(name, &raw_json) else {
        return Vec::new();
    };

    let resolved_name = info.tool_name.as_str();
    let args = info.args.as_ref();

    if args.is_none() {
        return vec![section("Payload", &info.raw)];
    }

    let args = args.unwrap();
    let mut out = match resolved_name {
        "bash_run" => {
            let mut out = Vec::new();
            if let Some(cmd) = arg_str(args, "command") {
                out.push(section("Command", cmd));
            }
            if let Some(wd) = arg_str(args, "workdir") {
                out.push(section("Working directory", wd));
            }
            out
        }
        "python_run" => arg_str(args, "code")
            .map(|code| vec![section("Python code", code)])
            .unwrap_or_default(),
        "write_file" => {
            let mut out = Vec::new();
            if let Some(path) = arg_str(args, "path") {
                out.push(section("Path", path));
            }
            if let Some(content) = arg_str(args, "content") {
                out.push(section("Content", content));
            }
            out
        }
        "edit_file" => {
            let mut out = Vec::new();
            if let Some(path) = arg_str(args, "path") {
                out.push(section("Path", path));
            }
            if let Some(old) = arg_str(args, "old_string") {
                out.push(section("Find", old));
            }
            if let Some(new) = arg_str(args, "new_string") {
                out.push(section("Replace with", new));
            }
            out
        }
        "pr_post_comment" => arg_str(args, "body")
            .map(|body| vec![section("Comment body", body)])
            .unwrap_or_default(),
        "ci_rerun_workflow" => vec![section(
            "Details",
            &format!(
                "repo: {}\nrun_id: {}",
                arg_str(args, "repo").unwrap_or("?"),
                args.get("run_id")
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "?".into())
            ),
        )],
        "pr_create_backport" => vec![section(
            "Details",
            &format!(
                "repo: {}\nPR: #{}\ntarget: {}",
                arg_str(args, "repo").unwrap_or("?"),
                args.get("pr_number")
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "?".into()),
                arg_str(args, "target_branch").unwrap_or("?")
            ),
        )],
        "issue_add_label" => vec![section(
            "Details",
            &format!(
                "repo: {}\nissue: #{}\nlabel: {}",
                arg_str(args, "repo").unwrap_or("?"),
                args.get("issue_number")
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "?".into()),
                arg_str(args, "label").unwrap_or("?")
            ),
        )],
        _ => vec![section("Arguments", &pretty_json(args))],
    };
    out.retain(|s| !s.text.trim().is_empty());
    if out.is_empty() {
        out.push(section("Arguments", &pretty_json(args)));
    }
    out
}

fn synthesize_tool_args_from_approval(approval: &Approval) -> Option<String> {
    match approval.kind {
        ApprovalKind::RerunFlaky => {
            let mut obj = serde_json::Map::new();
            obj.insert("repo".into(), approval.repo.clone().into());
            if let Some(run_id) = approval.run_id {
                obj.insert("run_id".into(), run_id.into());
            }
            Some(Value::Object(obj).to_string())
        }
        ApprovalKind::Backport => {
            let mut obj = serde_json::Map::new();
            obj.insert("repo".into(), approval.repo.clone().into());
            if let Some(pr) = approval.pr_number {
                obj.insert("pr_number".into(), pr.into());
            }
            if let Some(branch) = &approval.target_branch {
                obj.insert("target_branch".into(), branch.clone().into());
            }
            Some(Value::Object(obj).to_string())
        }
        ApprovalKind::IssueAddLabel => {
            let mut obj = serde_json::Map::new();
            obj.insert("repo".into(), approval.repo.clone().into());
            if let Some(issue) = approval.issue_number {
                obj.insert("issue_number".into(), issue.into());
            }
            if let Some(label) = &approval.label {
                obj.insert("label".into(), label.clone().into());
            }
            Some(Value::Object(obj).to_string())
        }
        ApprovalKind::PostComment => approval.comment_body.clone(),
        _ => approval.comment_body.clone(),
    }
}

struct ParsedApprovalArgs {
    tool_name: String,
    args: Option<Value>,
    raw: String,
}

fn parse_approval_args(tool_name: &str, tool_args_json: &str) -> Option<ParsedApprovalArgs> {
    let raw = tool_args_json.trim();
    if raw.is_empty() {
        return None;
    }
    match serde_json::from_str::<Value>(raw) {
        Ok(parsed) => {
            if let Some(obj) = parsed.as_object() {
                if let (Some(name), Some(args)) = (
                    obj.get("tool_name").and_then(|v| v.as_str()),
                    obj.get("args"),
                ) {
                    return Some(ParsedApprovalArgs {
                        tool_name: name.to_string(),
                        args: Some(args.clone()),
                        raw: raw.to_string(),
                    });
                }
            }
            Some(ParsedApprovalArgs {
                tool_name: tool_name.to_string(),
                args: Some(parsed),
                raw: raw.to_string(),
            })
        }
        Err(_) => Some(ParsedApprovalArgs {
            tool_name: tool_name.to_string(),
            args: None,
            raw: raw.to_string(),
        }),
    }
}

fn section(label: &str, text: &str) -> ApprovalPayloadSection {
    ApprovalPayloadSection {
        label: label.to_string(),
        text: text.to_string(),
    }
}

fn arg_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(|v| v.as_str())
}

fn pretty_json(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use uuid::Uuid;

    #[test]
    fn bash_run_shows_command() {
        let json = r#"{"command":"ls -la","workdir":"/tmp"}"#;
        let sections = build_approval_payload_sections("bash_run", Some(json));
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].label, "Command");
        assert_eq!(sections[0].text, "ls -la");
        assert_eq!(sections[1].label, "Working directory");
        assert_eq!(sections[1].text, "/tmp");
    }

    #[test]
    fn python_run_shows_code() {
        let json = r#"{"code":"print('hi')"}"#;
        let sections = build_approval_payload_sections("python_run", Some(json));
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].label, "Python code");
        assert_eq!(sections[0].text, "print('hi')");
    }

    #[test]
    fn mcp_tool_unwraps_nested_args() {
        let json = r#"{"tool_name":"remote_write","args":{"path":"a.txt","content":"x"}}"#;
        let sections = build_approval_payload_sections("mcp_tool", Some(json));
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].label, "Arguments");
        assert!(sections[0].text.contains("\"path\": \"a.txt\""));
    }

    #[test]
    fn post_comment_plain_body_from_store() {
        let sections = build_approval_payload_sections("pr_post_comment", Some("hello world"));
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].label, "Comment body");
        assert_eq!(sections[0].text, "hello world");
    }

    #[test]
    fn synthesize_rerun_from_approval_fields() {
        let approval = Approval {
            id: Uuid::new_v4(),
            kind: ApprovalKind::RerunFlaky,
            repo: "acme/widget".into(),
            pr_number: Some(42),
            run_id: Some(99),
            target_branch: None,
            incident_id: None,
            description: "rerun".into(),
            status: crate::store::ApprovalStatus::Pending,
            created_at: Utc::now(),
            decided_at: None,
            comment_body: None,
            issue_number: None,
            label: None,
        };
        let sections = build_approval_payload_sections_with_approval(
            "ci_rerun_workflow",
            None,
            Some(&approval),
        );
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].label, "Details");
        assert!(sections[0].text.contains("acme/widget"));
        assert!(sections[0].text.contains("run_id: 99"));
    }

    #[test]
    fn write_file_shows_path_and_content() {
        let json = r#"{"path":"src/main.rs","content":"fn main() {}"}"#;
        let sections = build_approval_payload_sections("write_file", Some(json));
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].label, "Path");
        assert_eq!(sections[1].label, "Content");
    }
}
