use chrono::Utc;
use uuid::Uuid;

use crate::app::append_audit;
use crate::config::Config;
use crate::engine::AgentSpec;
use crate::error::{CoworkerError, Result};
use crate::mcp::gh_query::list_merged_prs_labeled;
use crate::mcp::helpers::lazy_tool;
use crate::mcp::McpClient;
use crate::store::{
    Approval, ApprovalKind, ApprovalStatus, BackportQueueItem, BackportStatus, Store,
};

pub struct ReleaseOutcome {
    pub queued: u32,
    pub skipped: u32,
    pub body_md: String,
}

pub async fn run_release_duty(
    config: &Config,
    mcp: &dyn McpClient,
    store: &dyn Store,
    agent: &AgentSpec,
    log: impl Fn(&str, String),
) -> Result<ReleaseOutcome> {
    if config.release.target_branches.is_empty() {
        return Err(CoworkerError::Workflow(
            "release.target_branches is empty — set branches in coworker.yaml".into(),
        ));
    }

    log(
        "info",
        format!(
            "loaded agent '{}' — label `{}` → {:?}",
            agent.name, config.release.backport_label, config.release.target_branches
        ),
    );

    let mut queued = 0u32;
    let mut skipped = 0u32;
    let mut body = format!(
        "# Release / Backport duty\n\nAgent: {}\n\nLabel: `{}`\n\n",
        if agent.name.is_empty() {
            "release-duty"
        } else {
            &agent.name
        },
        config.release.backport_label
    );

    for repo in &config.repos.clone() {
        body.push_str(&format!("## {repo}\n\n"));
        log(
            "info",
            format!(
                "scanning merged PRs labeled `{}` in {repo}",
                config.release.backport_label
            ),
        );

        let prs = list_merged_prs_labeled(
            repo,
            &config.release.backport_label,
            config.release.lookback_limit,
        )
        .await?;

        if prs.is_empty() {
            body.push_str("_No merged PRs with backport label._\n\n");
            continue;
        }

        for pr in prs {
            let status = lazy_tool(
                mcp,
                "pr_get_status",
                serde_json::json!({ "repo": repo, "pr_number": pr.number }),
            )
            .await
            .unwrap_or_default();

            if !status.to_ascii_lowercase().contains("state: merged") {
                log(
                    "warn",
                    format!("PR #{}/{} not merged — skipping", repo, pr.number),
                );
                skipped += 1;
                body.push_str(&format!(
                    "- #{} {} — skipped (not merged per MCP)\n",
                    pr.number, pr.title
                ));
                continue;
            }

            for branch in &config.release.target_branches {
                if already_queued(store, repo, pr.number, branch).await? {
                    skipped += 1;
                    body.push_str(&format!(
                        "- #{} → `{}` — already queued\n",
                        pr.number, branch
                    ));
                    continue;
                }

                if config.policy.auto_backport {
                    log(
                        "info",
                        format!(
                            "auto_backport enabled — would backport #{}/{} → {branch}",
                            repo, pr.number
                        ),
                    );
                    skipped += 1;
                    continue;
                }

                let queue_id = Uuid::new_v4();
                let now = Utc::now();
                store
                    .upsert_backport_queue(&BackportQueueItem {
                        id: queue_id,
                        repo: repo.clone(),
                        pr_number: pr.number,
                        pr_title: pr.title.clone(),
                        target_branch: branch.clone(),
                        status: BackportStatus::Queued,
                        created_at: now,
                        updated_at: now,
                    })
                    .await?;

                store
                    .push_approval(&Approval {
                        id: Uuid::new_v4(),
                        kind: ApprovalKind::Backport,
                        repo: repo.clone(),
                        pr_number: Some(pr.number),
                        run_id: None,
                        target_branch: Some(branch.clone()),
                        incident_id: None,
                        description: format!(
                            "Backport merged PR #{} `{}` → `{branch}`?",
                            pr.number, pr.title
                        ),
                        status: ApprovalStatus::Pending,
                        created_at: now,
                        decided_at: None,
                        comment_body: None,
                    })
                    .await?;

                queued += 1;
                body.push_str(&format!(
                    "- #{} `{}` → `{}` — queued for approval\n",
                    pr.number, pr.title, branch
                ));
            }
        }
    }

    body.push_str(&format!("\nSummary: {queued} queued, {skipped} skipped\n"));

    append_audit(
        store,
        "info",
        "release-duty",
        &format!("backport queue: {queued} new, {skipped} skipped"),
    )
    .await;

    Ok(ReleaseOutcome {
        queued,
        skipped,
        body_md: body,
    })
}

async fn already_queued(
    store: &dyn Store,
    repo: &str,
    pr_number: u32,
    branch: &str,
) -> Result<bool> {
    let pending = store.list_pending_approvals().await?;
    if pending.iter().any(|a| {
        a.kind == ApprovalKind::Backport
            && a.repo == repo
            && a.pr_number == Some(pr_number)
            && a.target_branch.as_deref() == Some(branch)
    }) {
        return Ok(true);
    }

    let queue = store.list_backport_queue(Some(repo)).await?;
    Ok(queue.iter().any(|q| {
        q.pr_number == pr_number
            && q.target_branch == branch
            && !matches!(q.status, BackportStatus::Skipped | BackportStatus::Failed)
    }))
}
