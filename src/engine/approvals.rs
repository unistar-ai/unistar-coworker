use std::sync::Arc;

use serde_json::json;
use uuid::Uuid;

use crate::app::append_audit;
use crate::error::{CoworkerError, Result};
use crate::mcp::helpers::lazy_tool;
use crate::mcp::McpClient;
use crate::store::{ApprovalKind, BackportStatus, RerunOutcome, Store};

pub async fn process_decision(
    store: Arc<dyn Store>,
    mcp: Arc<dyn McpClient>,
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
        ApprovalKind::RerunFlaky => execute_rerun(mcp.as_ref(), &item).await,
        ApprovalKind::Backport => execute_backport(mcp.as_ref(), &item).await,
        ApprovalKind::PostComment => execute_post_comment(mcp.as_ref(), &item).await,
    };

    match exec_result {
        Ok(detail) => {
            store.decide_approval(id, true).await?;
            if let Some(incident_id) = item.incident_id {
                store
                    .update_flaky_rerun(&incident_id, RerunOutcome::Succeeded)
                    .await?;
            }
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
            if let Some(incident_id) = item.incident_id {
                let _ = store
                    .update_flaky_rerun(&incident_id, RerunOutcome::Failed)
                    .await;
            }
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

async fn execute_rerun(mcp: &dyn McpClient, item: &crate::store::Approval) -> Result<String> {
    let run_id = item
        .run_id
        .ok_or_else(|| CoworkerError::Workflow("rerun approval missing run_id".into()))?;
    let repo = crate::agent::chat_loop::sanitize_repo_string(&item.repo);
    let output = lazy_tool(
        mcp,
        "ci_rerun_workflow",
        json!({ "repo": repo, "run_id": run_id }),
    )
    .await?;
    if output.to_ascii_lowercase().contains("error") {
        return Err(CoworkerError::Workflow(format!(
            "ci_rerun_workflow failed: {output}"
        )));
    }
    Ok(format!("rerun triggered for run {run_id}: {output}"))
}

async fn execute_backport(mcp: &dyn McpClient, item: &crate::store::Approval) -> Result<String> {
    let pr_number = item
        .pr_number
        .ok_or_else(|| CoworkerError::Workflow("backport approval missing pr_number".into()))?;
    let target_branch = item
        .target_branch
        .as_deref()
        .ok_or_else(|| CoworkerError::Workflow("backport approval missing target_branch".into()))?;
    let output = lazy_tool(
        mcp,
        "pr_create_backport",
        json!({
            "repo": crate::agent::chat_loop::sanitize_repo_string(&item.repo),
            "pr_number": pr_number,
            "target_branch": target_branch,
        }),
    )
    .await?;
    if output.to_ascii_lowercase().contains("error") {
        return Err(CoworkerError::Workflow(format!(
            "pr_create_backport failed: {output}"
        )));
    }
    Ok(format!(
        "backport PR #{pr_number} → {target_branch}: {output}"
    ))
}

async fn execute_post_comment(
    mcp: &dyn McpClient,
    item: &crate::store::Approval,
) -> Result<String> {
    let pr_number = item
        .pr_number
        .ok_or_else(|| CoworkerError::Workflow("post comment approval missing pr_number".into()))?;
    let body = item.comment_body.as_deref().ok_or_else(|| {
        CoworkerError::Workflow("post comment approval missing comment_body".into())
    })?;
    let output = lazy_tool(
        mcp,
        "pr_post_comment",
        json!({
            "repo": crate::agent::chat_loop::sanitize_repo_string(&item.repo),
            "pr_number": pr_number,
            "body": body,
        }),
    )
    .await?;
    if output.to_ascii_lowercase().contains("error") {
        return Err(CoworkerError::Workflow(format!(
            "pr_post_comment failed: {output}"
        )));
    }
    Ok(format!(
        "comment posted on {}/#{}: {output}",
        item.repo, pr_number
    ))
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
