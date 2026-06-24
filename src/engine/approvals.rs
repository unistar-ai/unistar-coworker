use std::sync::Arc;

use serde_json::json;
use uuid::Uuid;

use crate::agent::bash_tool;
use crate::agent::file_tools;
use crate::agent::python_tool;
use crate::config::{BashToolConfig, PythonToolConfig};
use crate::agent::bash_tool::BASH_RUN_TOOL;
use crate::app::append_audit;
use crate::error::{CoworkerError, Result};
use crate::github::helpers::{gh_tool, mcp_text_indicates_failure};
use crate::github::GithubHarness;
use crate::mcp::McpPool;
use crate::store::{ApprovalKind, BackportStatus, Store};

pub async fn process_decision(
    store: Arc<dyn Store>,
    github: Arc<GithubHarness>,
    mcp: Arc<McpPool>,
    id: &Uuid,
    approve: bool,
) -> Result<String> {
    let item = store.get_pending_approval(id).await?;

    if !approve {
        store.decide_approval(id, false).await?;
        append_audit(
            store.as_ref(),
            "info",
            "approval",
            &format!("denied {:?} {}", item.kind, item.description),
        )
        .await;
        if item.kind == ApprovalKind::Backport {
            mark_backport_status(store.as_ref(), &item, BackportStatus::Skipped).await?;
        }
        return Ok("denied".into());
    }

    let exec_result = match item.kind {
        ApprovalKind::RerunFlaky => execute_rerun(github.as_ref(), &item).await,
        ApprovalKind::Backport => execute_backport(github.as_ref(), &item).await,
        ApprovalKind::PostComment => execute_post_comment(github.as_ref(), &item).await,
        ApprovalKind::IssueAddLabel => execute_issue_add_label(github.as_ref(), &item).await,
        ApprovalKind::WriteFile => execute_file_mutation(&item, file_tools::WRITE_FILE).await,
        ApprovalKind::EditFile => execute_file_mutation(&item, file_tools::EDIT_FILE).await,
        ApprovalKind::BashRun => execute_bash_run_approval(&item).await,
        ApprovalKind::PythonRun => execute_python_run_approval(&item).await,
        ApprovalKind::McpTool => execute_mcp_tool(mcp.as_ref(), &item).await,
    };

    match exec_result {
        Ok(detail) => {
            store.decide_approval(id, true).await?;
            if item.kind == ApprovalKind::Backport {
                mark_backport_status(store.as_ref(), &item, BackportStatus::Created).await?;
            }
            append_audit(
                store.as_ref(),
                "info",
                "approval",
                &format!("approved {:?}: {detail}", item.kind),
            )
            .await;
            Ok(detail)
        }
        Err(e) => {
            if item.kind == ApprovalKind::Backport {
                let _ = mark_backport_status(store.as_ref(), &item, BackportStatus::Failed).await;
            }
            append_audit(
                store.as_ref(),
                "error",
                "approval",
                &format!("failed {:?}: {e}", item.kind),
            )
            .await;
            Err(e)
        }
    }
}

async fn execute_rerun(mcp: &GithubHarness, item: &crate::store::Approval) -> Result<String> {
    let run_id = item
        .run_id
        .ok_or_else(|| CoworkerError::Workflow("rerun approval missing run_id".into()))?;
    let repo = crate::agent::chat_loop::sanitize_repo_string(&item.repo);
    let output = gh_tool(
        mcp,
        "ci_rerun_workflow",
        json!({ "repo": repo, "run_id": run_id }),
    )
    .await?;
    if mcp_text_indicates_failure(&output) {
        return Err(CoworkerError::Workflow(format!(
            "ci_rerun_workflow failed: {output}"
        )));
    }
    Ok(format!("rerun triggered for run {run_id}: {output}"))
}

async fn execute_backport(mcp: &GithubHarness, item: &crate::store::Approval) -> Result<String> {
    let pr_number = item
        .pr_number
        .ok_or_else(|| CoworkerError::Workflow("backport approval missing pr_number".into()))?;
    let target_branch = item
        .target_branch
        .as_deref()
        .ok_or_else(|| CoworkerError::Workflow("backport approval missing target_branch".into()))?;
    let output = gh_tool(
        mcp,
        "pr_create_backport",
        json!({
            "repo": crate::agent::chat_loop::sanitize_repo_string(&item.repo),
            "pr_number": pr_number,
            "target_branch": target_branch,
        }),
    )
    .await?;
    if mcp_text_indicates_failure(&output) {
        return Err(CoworkerError::Workflow(format!(
            "pr_create_backport failed: {output}"
        )));
    }
    Ok(format!(
        "backport PR #{pr_number} → {target_branch}: {output}"
    ))
}

async fn execute_post_comment(
    mcp: &GithubHarness,
    item: &crate::store::Approval,
) -> Result<String> {
    let pr_number = item
        .pr_number
        .ok_or_else(|| CoworkerError::Workflow("post comment approval missing pr_number".into()))?;
    let body = item.comment_body.as_deref().ok_or_else(|| {
        CoworkerError::Workflow("post comment approval missing comment_body".into())
    })?;
    let output = gh_tool(
        mcp,
        "pr_post_comment",
        json!({
            "repo": crate::agent::chat_loop::sanitize_repo_string(&item.repo),
            "pr_number": pr_number,
            "body": body,
        }),
    )
    .await?;
    if mcp_text_indicates_failure(&output) {
        return Err(CoworkerError::Workflow(format!(
            "pr_post_comment failed: {output}"
        )));
    }
    Ok(format!(
        "comment posted on {}/#{}: {output}",
        item.repo, pr_number
    ))
}

async fn execute_issue_add_label(
    mcp: &GithubHarness,
    item: &crate::store::Approval,
) -> Result<String> {
    let issue_number = item.issue_number.ok_or_else(|| {
        CoworkerError::Workflow("issue label approval missing issue_number".into())
    })?;
    let label = item.label.as_deref().ok_or_else(|| {
        CoworkerError::Workflow("issue label approval missing label".into())
    })?;
    let output = gh_tool(
        mcp,
        "issue_add_label",
        json!({
            "repo": crate::agent::chat_loop::sanitize_repo_string(&item.repo),
            "issue_number": issue_number,
            "label": label,
        }),
    )
    .await?;
    if mcp_text_indicates_failure(&output) {
        return Err(CoworkerError::Workflow(format!(
            "issue_add_label failed: {output}"
        )));
    }
    Ok(format!(
        "label `{label}` added to {}/issue/{issue_number}: {output}",
        item.repo
    ))
}

async fn execute_mcp_tool(mcp: &McpPool, item: &crate::store::Approval) -> Result<String> {
    let payload = item.comment_body.as_deref().ok_or_else(|| {
        CoworkerError::Workflow("mcp tool approval missing args payload".into())
    })?;
    let parsed: serde_json::Value = serde_json::from_str(payload).map_err(|e| {
        CoworkerError::Workflow(format!("mcp tool approval args invalid JSON: {e}"))
    })?;
    let tool_name = parsed
        .get("tool_name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| CoworkerError::Workflow("mcp tool approval missing tool_name".into()))?;
    let args = parsed
        .get("args")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    let output = mcp.call_global_tool_approved(tool_name, args).await?;
    Ok(format!("{tool_name} approved: {output}"))
}

async fn execute_file_mutation(item: &crate::store::Approval, tool_name: &str) -> Result<String> {
    let payload = item.comment_body.as_deref().ok_or_else(|| {
        CoworkerError::Workflow(format!("{tool_name} approval missing args payload"))
    })?;
    let args: serde_json::Value = serde_json::from_str(payload).map_err(|e| {
        CoworkerError::Workflow(format!("{tool_name} approval args invalid JSON: {e}"))
    })?;
    let workspace = std::path::PathBuf::from(&item.repo);
    let output = file_tools::execute_mutating_file_tool(&workspace, tool_name, &args)?;
    Ok(format!("{tool_name} approved: {output}"))
}

async fn execute_bash_run_approval(item: &crate::store::Approval) -> Result<String> {
    let args = approval_args_json(item, BASH_RUN_TOOL)?;
    let workspace = std::path::PathBuf::from(&item.repo);
    let config = BashToolConfig::default();
    let output = bash_tool::execute_bash_approved(&config, &workspace, &args).await?;
    Ok(format!("bash_run approved: {output}"))
}

async fn execute_python_run_approval(item: &crate::store::Approval) -> Result<String> {
    let args = approval_args_json(item, python_tool::PYTHON_RUN_TOOL)?;
    let workspace = std::path::PathBuf::from(&item.repo);
    let config = PythonToolConfig::default();
    let output = python_tool::execute_python_approved(&config, &workspace, &args).await?;
    Ok(format!("python_run approved: {output}"))
}

fn approval_args_json(item: &crate::store::Approval, tool_name: &str) -> Result<serde_json::Value> {
    let payload = item.comment_body.as_deref().ok_or_else(|| {
        CoworkerError::Workflow(format!("{tool_name} approval missing args payload"))
    })?;
    serde_json::from_str(payload).map_err(|e| {
        CoworkerError::Workflow(format!("{tool_name} approval args invalid JSON: {e}"))
    })
}

async fn mark_backport_status(
    store: &dyn Store,
    item: &crate::store::Approval,
    status: BackportStatus,
) -> Result<()> {
    let pr_number = match item.pr_number {
        Some(n) => n,
        None => return Ok(()),
    };
    let target_branch = match &item.target_branch {
        Some(b) => b.clone(),
        None => return Ok(()),
    };
    let queue = store.list_backport_queue(Some(&item.repo)).await?;
    if let Some(mut entry) = queue
        .into_iter()
        .find(|q| q.pr_number == pr_number && q.target_branch == target_branch)
    {
        entry.status = status;
        entry.updated_at = chrono::Utc::now();
        store.upsert_backport_queue(&entry).await?;
    }
    Ok(())
}
