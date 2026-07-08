//! Workflow delegate tools — run batch workflows from chat (aligns with TUI shortcuts).

use std::sync::Arc;

use serde_json::{json, Value};
use tokio::sync::{broadcast, RwLock};

use crate::agent::parse::{parse_pr_line, ParsedPrLine};
use crate::agent::r#loop::AgentLoop;
use crate::agent::triage::triage_pr;
use crate::app::AppState;
use crate::config::Config;
use crate::engine::{load_classify_skills_for_triage, require_workflow};
use crate::error::{CoworkerError, Result};
use crate::github::helpers::gh_tool;
use crate::github::GithubHarness;
use crate::llm::LlmClient;
use crate::store::Store;

pub const WORKFLOW_HARNESS_TOOLS: &[&str] = &[
    "harness_triage_pr",
    "harness_run_workflow",
    "harness_daily_digest",
];

pub fn is_workflow_harness_tool(name: &str) -> bool {
    WORKFLOW_HARNESS_TOOLS.contains(&name)
}

pub struct WorkflowHarnessCtx {
    pub config: Arc<Config>,
    pub store: Arc<dyn Store>,
    pub github: Arc<GithubHarness>,
    pub llm: Arc<LlmClient>,
}

pub async fn execute_workflow_harness(
    ctx: WorkflowHarnessCtx,
    name: &str,
    args: Value,
) -> Result<String> {
    match name {
        "harness_triage_pr" => harness_triage_pr(ctx, args).await,
        "harness_run_workflow" => harness_run_workflow(ctx, args).await,
        "harness_daily_digest" => {
            harness_run_workflow(ctx, json!({ "workflow_id": "daily-work" })).await
        }
        other => Err(CoworkerError::Workflow(format!(
            "unknown workflow harness tool: {other}"
        ))),
    }
}

async fn harness_triage_pr(ctx: WorkflowHarnessCtx, args: Value) -> Result<String> {
    if !ctx.github.is_available() {
        return Err(CoworkerError::Workflow(
            "harness_triage_pr requires GitHub harness".into(),
        ));
    }
    let repo = args
        .get("repo")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| CoworkerError::Workflow("harness_triage_pr needs repo".into()))?;
    let pr_number = args
        .get("pr_number")
        .and_then(|v| v.as_u64())
        .map(|n| n as u32)
        .ok_or_else(|| CoworkerError::Workflow("harness_triage_pr needs pr_number".into()))?;

    let pr_line = resolve_pr_line(ctx.github.as_ref(), repo, pr_number).await?;
    let classify_skills = load_classify_skills_for_triage(&[])?;
    let outcome = triage_pr(
        ctx.config.as_ref(),
        ctx.github.as_ref(),
        ctx.llm.as_ref(),
        ctx.store.as_ref(),
        &classify_skills,
        repo,
        &pr_line,
        None,
    )
    .await?;

    let note = outcome.full_note();
    Ok(format!(
        "harness_triage_pr {repo}#{pr_number} complete\n\n{note}\n\n\
(local PrSnapshot.triage_note updated — same as TUI `t` on PRs tab)"
    ))
}

async fn harness_run_workflow(ctx: WorkflowHarnessCtx, args: Value) -> Result<String> {
    if !ctx.github.is_available() {
        return Err(CoworkerError::Workflow(
            "harness_run_workflow requires GitHub harness".into(),
        ));
    }
    let workflow_id = args
        .get("workflow_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| CoworkerError::Workflow("harness_run_workflow needs workflow_id".into()))?;
    require_workflow(workflow_id)?;

    let (events, _) = broadcast::channel(32);
    let state = Arc::new(RwLock::new(AppState::new(
        ctx.config.as_ref().clone(),
        String::new(),
    )));
    let agent = AgentLoop::new(
        ctx.config.as_ref().clone(),
        Arc::clone(&ctx.store),
        Arc::clone(&ctx.github),
        Arc::clone(&ctx.llm),
        events,
        state,
    );
    let summary = agent.run_workflow(workflow_id).await?;
    Ok(format!(
        "harness_run_workflow {workflow_id} complete\n\n{summary}\n\n\
(same workflow as TUI / scheduler / `run-once --workflow {workflow_id}`)"
    ))
}

async fn resolve_pr_line(
    github: &GithubHarness,
    repo: &str,
    pr_number: u32,
) -> Result<ParsedPrLine> {
    let list_text = gh_tool(github, "pr_list_open", json!({ "repo": repo, "limit": 50 })).await?;
    if let Some(p) = list_text.lines().find_map(|line| {
        let p = parse_pr_line(line)?;
        (p.number == pr_number).then_some(p)
    }) {
        return Ok(p);
    }
    Err(CoworkerError::Workflow(format!(
        "PR #{pr_number} not found in {repo}"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workflow_harness_tool_registry() {
        assert!(is_workflow_harness_tool("harness_triage_pr"));
        assert!(is_workflow_harness_tool("harness_run_workflow"));
        assert!(is_workflow_harness_tool("harness_daily_digest"));
        assert!(!is_workflow_harness_tool("pr_get_overview"));
    }
}
