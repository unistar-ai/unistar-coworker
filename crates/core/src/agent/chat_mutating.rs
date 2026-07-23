use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use chrono::Utc;
use serde_json::{json, Value};
use tokio::sync::{broadcast, Mutex};
use uuid::Uuid;

use crate::agent::budget::TokenBudget;
use crate::agent::chat_discovery::ChatDiscoveryState;
use crate::agent::context::{format_tool_approval_pending_message, format_tool_context_message};
use crate::agent::file_tools;
use crate::agent::review_gate::format_review_rejection_description;
use crate::app::{append_audit, AppEvent};
use crate::config::{ChatToolMode, Config};
use crate::engine::approvals;
use crate::error::{CoworkerError, Result};
use crate::github::GithubHarness;
use crate::llm::chat::ChatAgentStep;
use crate::llm::LlmTurnMessage;
use crate::mcp::McpPool;
use crate::store::{Approval, ApprovalKind, ApprovalStatus, Store};

use crate::agent::chat_loop::{
    append_tool_result_message, emit_context_snapshot, emit_progress,
    persist_native_assistant_tool_call_step, record_session_file_edit, sanitize_repo_string,
    store_update_session_runtime, ChatProgress, PreparedToolCall, ToolCallSummary,
};

pub(crate) enum MutatingToolOutcome {
    Continue,
    AwaitingApproval,
}

pub(crate) struct MutatingToolContext<'a> {
    pub store: &'a dyn Store,
    pub session_id: &'a Uuid,
    pub session: &'a mut crate::store::ChatSession,
    pub workspace: &'a Path,
    pub step: &'a ChatAgentStep,
    pub config: &'a Config,
    pub store_arc: &'a Arc<dyn Store>,
    pub github: &'a Arc<GithubHarness>,
    pub mcp: &'a Arc<McpPool>,
    pub progress: &'a Option<broadcast::Sender<AppEvent>>,
    pub llm_messages: &'a mut Vec<LlmTurnMessage>,
    pub tool_calls: &'a mut Vec<ToolCallSummary>,
    pub llm_rounds: u32,
    pub token_budget: &'a TokenBudget,
    pub discovery: Arc<Mutex<ChatDiscoveryState>>,
    pub tool_mode: ChatToolMode,
    pub runtime_panel: (String, u64),
    pub reasoning_originals: &'a HashMap<String, String>,
}

pub(crate) async fn handle_mutating_tool_call(
    ctx: MutatingToolContext<'_>,
    call: &PreparedToolCall,
) -> Result<MutatingToolOutcome> {
    let tool_name = call.name.as_str();
    let tool_args = &call.args;
    let queued =
        queue_mutating_approval(ctx.store, ctx.workspace, ctx.mcp, tool_name, tool_args).await?;
    if let Some(detail) = maybe_auto_approve_mutations(
        ctx.config,
        ctx.store_arc,
        ctx.github,
        ctx.mcp,
        tool_name,
        &queued,
    )
    .await?
    {
        if file_tools::is_mutating_file_tool(tool_name) {
            record_session_file_edit(ctx.session, tool_name, tool_args, &detail);
            store_update_session_runtime(ctx.store, ctx.session).await?;
        }
        emit_progress(
            ctx.progress,
            ChatProgress::ApprovalResolved {
                approval_id: queued.id,
                tool_name: queued.tool_name.clone(),
                approved: true,
                detail: detail.clone(),
            },
        );
        ctx.tool_calls.push(ToolCallSummary {
            tool_name: format!("approval:{}", queued.tool_name),
            output: detail.clone(),
        });
        let body = format_tool_context_message(
            tool_name,
            tool_args,
            true,
            &format!("Auto-approved: {detail}"),
        );
        ctx.llm_messages.push(LlmTurnMessage::tool_result_with_id(
            Some(call.id.clone()),
            tool_name,
            body.clone(),
        ));
        append_tool_result_message(
            ctx.store,
            ctx.session_id,
            &body,
            tool_name,
            tool_args.to_string(),
            Some(call.id.as_str()),
        )
        .await?;
        return Ok(MutatingToolOutcome::Continue);
    }
    persist_native_assistant_tool_call_step(ctx.store, ctx.session_id, ctx.step).await?;
    emit_progress(
        ctx.progress,
        ChatProgress::ApprovalQueued {
            approval_id: queued.id,
            session_id: *ctx.session_id,
            tool_name: queued.tool_name.clone(),
            tool_args_json: tool_args.to_string(),
            description: queued.description.clone(),
            tool_call_id: call.id.clone(),
        },
    );
    ctx.tool_calls.push(ToolCallSummary {
        tool_name: format!("approval:{}", queued.tool_name),
        output: queued.summary.clone(),
    });
    let pending_body = format!("Mutating tool awaiting approval. {}", queued.summary);
    let body = format_tool_approval_pending_message(tool_name, tool_args, queued.id, &pending_body);
    append_tool_result_message(
        ctx.store,
        ctx.session_id,
        &body,
        tool_name,
        tool_args.to_string(),
        Some(call.id.as_str()),
    )
    .await?;
    ctx.llm_messages.push(LlmTurnMessage::tool_result_with_id(
        Some(call.id.clone()),
        tool_name,
        body,
    ));
    emit_context_snapshot(
        ctx.progress,
        ctx.llm_messages,
        ctx.llm_rounds,
        ctx.token_budget,
        &ctx.discovery,
        ctx.tool_mode,
        Some((ctx.runtime_panel.0.as_str(), ctx.runtime_panel.1)),
        Some((ctx.store, *ctx.session_id)),
        ctx.reasoning_originals,
    )
    .await;
    Ok(MutatingToolOutcome::AwaitingApproval)
}

pub(crate) async fn queue_review_fallback_approval(
    store: &dyn Store,
    workspace: &Path,
    tool_name: &str,
    args: &Value,
    review: &crate::agent::bash_tool::BashCommandReview,
) -> Result<QueuedApproval> {
    use crate::agent::review_gate::approval_kind_for_review_gated_tool;

    let workspace_key = workspace.to_string_lossy().to_string();
    let args_json = serde_json::to_string(args).unwrap_or_else(|_| "{}".to_string());
    let kind = approval_kind_for_review_gated_tool(tool_name).ok_or_else(|| {
        CoworkerError::Workflow(format!(
            "no approval kind for review-gated tool: {tool_name}"
        ))
    })?;
    let description = format_review_rejection_description(tool_name, review);

    let approval = Approval {
        id: Uuid::new_v4(),
        kind,
        repo: workspace_key,
        pr_number: None,
        run_id: None,
        target_branch: None,
        incident_id: None,
        description: description.clone(),
        status: ApprovalStatus::Pending,
        created_at: Utc::now(),
        decided_at: None,
        decision_reason: None,
        comment_body: Some(args_json),
        issue_number: None,
        label: None,
    };
    store.push_approval(&approval).await?;
    append_audit(
        store,
        "info",
        "chat",
        &format!(
            "queued LLM-review fallback approval {} ({:?})",
            approval.id, approval.kind
        ),
    )
    .await;
    Ok(QueuedApproval {
        id: approval.id,
        tool_name: tool_name.to_string(),
        description: approval.description.clone(),
        summary: format!(
            "LLM safety review rejected `{tool_name}` — approval {} queued (confirm in Approvals UI).",
            approval.id
        ),
    })
}

pub(crate) async fn queue_mutating_approval(
    store: &dyn Store,
    workspace: &Path,
    mcp: &Arc<McpPool>,
    tool_name: &str,
    args: &Value,
) -> Result<QueuedApproval> {
    if mcp.is_mcp_mutating(tool_name).await {
        let entry = mcp
            .resolve_entry(tool_name)
            .await
            .ok_or_else(|| CoworkerError::Workflow(format!("unknown MCP tool {tool_name:?}")))?;
        let payload = serde_json::to_string(&json!({
            "tool_name": tool_name,
            "args": args,
        }))
        .map_err(|e| CoworkerError::Workflow(format!("mcp approval payload: {e}")))?;
        let workspace_repo = workspace.to_string_lossy().into_owned();
        let description = format!(
            "Chat: MCP {} on mcp[{}]",
            entry.remote_name, entry.server_id
        );
        let approval = Approval {
            id: Uuid::new_v4(),
            kind: ApprovalKind::McpTool,
            repo: workspace_repo,
            pr_number: None,
            run_id: None,
            target_branch: None,
            incident_id: None,
            description,
            status: ApprovalStatus::Pending,
            created_at: Utc::now(),
            decided_at: None,
            decision_reason: None,
            comment_body: Some(payload),
            issue_number: None,
            label: None,
        };
        store.push_approval(&approval).await?;
        append_audit(
            store,
            "info",
            "chat",
            &format!("queued approval {} ({:?})", approval.id, approval.kind),
        )
        .await;
        return Ok(QueuedApproval {
            id: approval.id,
            tool_name: tool_name.to_string(),
            description: approval.description.clone(),
            summary: format!(
                "Approval {} queued for `{tool_name}` — confirm in the approval popup.",
                approval.id
            ),
        });
    }

    let comment_body = args
        .get("body")
        .and_then(|v| v.as_str())
        .map(str::to_string);

    let (kind, description, repo, pr_number, run_id, target_branch, issue_number, label, payload) =
        match tool_name {
            "ci_rerun_workflow" => {
                let run_id = args.get("run_id").and_then(|v| v.as_i64()).ok_or_else(|| {
                    CoworkerError::Workflow("ci_rerun_workflow needs run_id".into())
                })?;
                let repo = args
                    .get("repo")
                    .and_then(|v| v.as_str())
                    .map(sanitize_repo_string)
                    .unwrap_or_else(|| "unknown/repo".to_string());
                (
                    ApprovalKind::RerunFlaky,
                    format!("Chat: rerun workflow run {run_id} on {repo}"),
                    repo,
                    None,
                    Some(run_id),
                    None,
                    None,
                    None,
                    None,
                )
            }
            "pr_create_backport" => {
                let repo = args
                    .get("repo")
                    .and_then(|v| v.as_str())
                    .map(sanitize_repo_string)
                    .unwrap_or_else(|| "unknown/repo".to_string());
                let pr_number = args
                    .get("pr_number")
                    .and_then(|v| v.as_u64())
                    .map(|n| n as u32)
                    .ok_or_else(|| {
                        CoworkerError::Workflow("pr_create_backport needs pr_number".into())
                    })?;
                let target_branch = args
                    .get("target_branch")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        CoworkerError::Workflow("pr_create_backport needs target_branch".into())
                    })?
                    .to_string();
                (
                    ApprovalKind::Backport,
                    format!("Chat: backport #{pr_number} → {target_branch} on {repo}"),
                    repo,
                    Some(pr_number),
                    None,
                    Some(target_branch),
                    None,
                    None,
                    None,
                )
            }
            "pr_post_comment" => {
                let repo = args
                    .get("repo")
                    .and_then(|v| v.as_str())
                    .map(sanitize_repo_string)
                    .unwrap_or_else(|| "unknown/repo".to_string());
                let pr_number = args
                    .get("pr_number")
                    .and_then(|v| v.as_u64())
                    .map(|n| n as u32)
                    .ok_or_else(|| {
                        CoworkerError::Workflow("pr_post_comment needs pr_number".into())
                    })?;
                if comment_body.is_none() {
                    return Err(CoworkerError::Workflow("pr_post_comment needs body".into()));
                }
                (
                    ApprovalKind::PostComment,
                    format!("Chat: post comment on #{pr_number} ({repo})"),
                    repo,
                    Some(pr_number),
                    None,
                    None,
                    None,
                    None,
                    None,
                )
            }
            "issue_add_label" => {
                let repo = args
                    .get("repo")
                    .and_then(|v| v.as_str())
                    .map(sanitize_repo_string)
                    .unwrap_or_else(|| "unknown/repo".to_string());
                let issue_number = args
                    .get("issue_number")
                    .and_then(|v| v.as_u64())
                    .map(|n| n as u32)
                    .ok_or_else(|| {
                        CoworkerError::Workflow("issue_add_label needs issue_number".into())
                    })?;
                let label = args
                    .get("label")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .ok_or_else(|| CoworkerError::Workflow("issue_add_label needs label".into()))?;
                (
                    ApprovalKind::IssueAddLabel,
                    format!("Chat: add label `{label}` to issue #{issue_number} ({repo})"),
                    repo,
                    None,
                    None,
                    None,
                    Some(issue_number),
                    Some(label),
                    None,
                )
            }
            other => {
                return Err(CoworkerError::Workflow(format!(
                    "unknown mutating tool: {other}"
                )));
            }
        };

    let approval = Approval {
        id: Uuid::new_v4(),
        kind,
        repo,
        pr_number,
        run_id,
        target_branch,
        incident_id: None,
        description,
        status: ApprovalStatus::Pending,
        created_at: Utc::now(),
        decided_at: None,
        decision_reason: None,
        comment_body: payload.or(comment_body),
        issue_number,
        label,
    };
    store.push_approval(&approval).await?;
    append_audit(
        store,
        "info",
        "chat",
        &format!("queued approval {} ({:?})", approval.id, approval.kind),
    )
    .await;
    Ok(QueuedApproval {
        id: approval.id,
        tool_name: tool_name.to_string(),
        description: approval.description.clone(),
        summary: format!(
            "Approval {} queued for `{tool_name}` — confirm in the approval popup.",
            approval.id
        ),
    })
}

/// When `chat.auto_approve_mutations` is enabled, run the queued mutation immediately.
pub(crate) async fn maybe_auto_approve_mutations(
    config: &Config,
    store: &Arc<dyn Store>,
    github: &Arc<GithubHarness>,
    mcp: &Arc<McpPool>,
    tool_name: &str,
    queued: &QueuedApproval,
) -> Result<Option<String>> {
    let auto = if config.chat.auto_approve_mutations {
        true
    } else if mcp.is_mcp_mutating(tool_name).await {
        matches!(
            mcp.server_mutating_policy(tool_name).await,
            Some(crate::config::McpMutatingPolicy::Auto)
        )
    } else {
        false
    };
    if !auto {
        return Ok(None);
    }
    match approvals::process_decision(
        Arc::clone(store),
        Arc::clone(github),
        Arc::clone(mcp),
        config,
        &queued.id,
        true,
        None,
    )
    .await
    {
        Ok(detail) => Ok(Some(detail)),
        Err(e) => Err(e),
    }
}

/// Result of queueing a mutating tool for human approval.
#[derive(Debug, Clone)]
pub struct QueuedApproval {
    pub id: Uuid,
    pub tool_name: String,
    pub description: String,
    pub summary: String,
}
