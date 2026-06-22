use std::time::Duration;

use serde_json::Value;

use super::backport;
use super::ci;
use super::ci_check_url;
use super::ci_digest;
use super::ci_fingerprint;
use super::ci_health;
use super::ci_tier2;
use super::ci_workflow_stats;
use super::discovery;
use super::error::not_implemented_yet;
use super::events;
use super::exec::GhExec;
use super::issue;
use super::notify;
use super::policy;
use super::pr;
use super::pr_batch;
use super::release;
use super::repo;
use super::resources;
use super::security;
use crate::config::GithubConfig;
use crate::error::{CoworkerError, Result};

pub struct GithubHarness {
    exec: GhExec,
    available: bool,
}

impl GithubHarness {
    pub async fn try_new(cfg: &GithubConfig) -> Self {
        let exec = GhExec {
            gh: cfg.gh_command.clone(),
            timeout: Duration::from_secs(cfg.timeout_secs.max(1)),
        };
        let probe = exec.run(&["--version"]).await;
        let available = probe.err.is_none();
        if !available {
            tracing::warn!(
                "`{}` not available — GitHub tools disabled; coding chat still works",
                cfg.gh_command
            );
        } else if exec.run(&["auth", "status"]).await.err.is_some() {
            tracing::warn!("GitHub CLI not authenticated — run `gh auth login` or set GH_TOKEN");
        } else {
            tracing::info!("GitHub harness ready ({})", cfg.gh_command);
        }
        Self { exec, available }
    }

    pub fn is_available(&self) -> bool {
        self.available
    }

    /// Harness always exposes lazy discovery meta-tools (no MCP subprocess).
    pub fn supports_lazy_meta(&self) -> bool {
        true
    }

    pub async fn call_tool(&self, name: &str, args: Value) -> Result<String> {
        if !self.available {
            return Err(CoworkerError::Other(anyhow::anyhow!(
                "GitHub harness unavailable — install `gh` and set GH_TOKEN"
            )));
        }
        match name {
            "tool_list" => Ok(discovery::tool_list()),
            "tool_list_category" => {
                let category = args
                    .get("category")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| CoworkerError::Workflow("tool_list_category needs category".into()))?;
                discovery::tool_list_category(category)
            }
            "tool_search" => {
                let query = args
                    .get("query")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| CoworkerError::Workflow("tool_search needs query".into()))?;
                let limit = args
                    .get("limit")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(5) as usize;
                discovery::tool_search(query, limit)
            }
            "tool_describe" => {
                let tool = args
                    .get("name")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| CoworkerError::Workflow("tool_describe needs name".into()))?;
                discovery::tool_describe(tool)
            }
            "tool_call" => {
                let inner = args
                    .get("name")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| CoworkerError::Workflow("tool_call needs name".into()))?;
                let inner_args = args.get("args").cloned().unwrap_or_else(|| Value::Object(Default::default()));
                return self.dispatch_tool(inner, inner_args).await;
            }
            other => self.dispatch_tool(other, args).await,
        }
    }

    async fn dispatch_tool(&self, name: &str, args: Value) -> Result<String> {
        match name {
            // PR
            "pr_list_open" => pr::pr_list_open(&self.exec, &args).await,
            "pr_get_status" => pr::pr_get_status(&self.exec, &args).await,
            "pr_get_overview" => pr::pr_get_overview(&self.exec, &args).await,
            "pr_list_changed_files" => pr::pr_list_changed_files(&self.exec, &args).await,
            "pr_list_stale" => pr::pr_list_stale(&self.exec, &args).await,
            "pr_list_merged" => pr::pr_list_merged(&self.exec, &args).await,
            "pr_get_diff" => pr::pr_get_diff(&self.exec, &args).await,
            "pr_post_comment" => pr::pr_post_comment(&self.exec, &args).await,
            "pr_get_merge_blockers" => pr::pr_get_merge_blockers(&self.exec, &args).await,
            "pr_list_waiting_review" => pr::pr_list_waiting_review(&self.exec, &args).await,
            "pr_list_merge_ready" => pr::pr_list_merge_ready(&self.exec, &args).await,
            "pr_list_merge_blocked" => pr::pr_list_merge_blocked(&self.exec, &args).await,
            "pr_list_large" => pr::pr_list_large(&self.exec, &args).await,
            "pr_list_backport_candidates" => pr::pr_list_backport_candidates(&self.exec, &args).await,
            "pr_is_docs_only" => pr::pr_is_docs_only(&self.exec, &args).await,
            "pr_get_review_state" => pr::pr_get_review_state(&self.exec, &args).await,
            "pr_diff_risk_scan" => pr::pr_diff_risk_scan(&self.exec, &args).await,
            "pr_get_review_routing" => pr::pr_get_review_routing(&self.exec, &args).await,
            "pr_draft_ci_comment" => pr::pr_draft_ci_comment(&self.exec, &args).await,
            "pr_get_status_batch" => pr_batch::pr_get_status_batch(&self.exec, &args).await,
            "pr_get_overview_batch" => pr_batch::pr_get_overview_batch(&self.exec, &args).await,
            "pr_get_ci_snapshot" => ci_digest::pr_get_ci_snapshot(&self.exec, &args).await,
            // CI
            "ci_analyze_pr_failures" => ci::ci_analyze_pr_failures(&self.exec, &args).await,
            "ci_get_run_summary" => ci::ci_get_run_summary(&self.exec, &args).await,
            "ci_get_failed_logs" => ci::ci_get_failed_logs(&self.exec, &args).await,
            "ci_list_runs" => ci::ci_list_runs(&self.exec, &args).await,
            "ci_rerun_workflow" => ci::ci_rerun_workflow(&self.exec, &args).await,
            "ci_get_failure_digest" => ci_digest::ci_get_failure_digest(&self.exec, &args).await,
            "ci_failure_fingerprint" => ci_fingerprint::ci_failure_fingerprint(&self.exec, &args).await,
            "ci_compare_runs" => ci_fingerprint::ci_compare_runs(&self.exec, &args).await,
            "ci_list_external_checks" => ci_fingerprint::ci_list_external_checks(&self.exec, &args).await,
            "ci_get_job_logs" => ci_tier2::ci_get_job_logs(&self.exec, &args).await,
            "ci_correlate_prs" => ci_tier2::ci_correlate_prs(&self.exec, &args).await,
            "ci_list_workflows" => ci_tier2::ci_list_workflows(&self.exec, &args).await,
            "ci_branch_health" => ci_health::ci_branch_health(&self.exec, &args).await,
            "ci_workflow_stats" => ci_workflow_stats::ci_workflow_stats(&self.exec, &args).await,
            "ci_get_check_url" => ci_check_url::ci_get_check_url(&self.exec, &args).await,
            "policy_classify_failure" => policy::policy_classify_failure(&self.exec, &args).await,
            // Repo / issue
            "repo_get_info" => repo::repo_get_info(&self.exec, &args).await,
            "issue_list_open" => issue::issue_list_open(&self.exec, &args).await,
            "issue_get" => issue::issue_get(&self.exec, &args).await,
            "issue_add_label" => issue::issue_add_label(&self.exec, &args).await,
            "issue_search" => issue::issue_search(&self.exec, &args).await,
            // Security / release / backport / notify / events
            "alert_list_open" => security::alert_list_open(&self.exec, &args).await,
            "alert_summarize_open" => security::alert_summarize_open(&self.exec, &args).await,
            "release_list_tags" => release::release_list_tags(&self.exec, &args).await,
            "release_notes_draft" => release::release_notes_draft(&self.exec, &args).await,
            "pr_create_backport" => backport::pr_create_backport(&self.exec, &args).await,
            "backport_get_conflict_files" => backport::backport_get_conflict_files(&self.exec, &args).await,
            "backport_suggest_resolution" => backport::backport_suggest_resolution(&self.exec, &args).await,
            "notify_post_slack" => notify::notify_post_slack(&self.exec, &args).await,
            "event_list_recent" => events::event_list_recent(&self.exec, &args).await,
            other => Err(not_implemented_yet(other)),
        }
    }

    pub async fn read_resource(&self, uri: &str) -> Result<String> {
        resources::read_resource_via_dispatch(uri, |tool, args| self.dispatch_tool(tool, args)).await
    }
}

pub async fn spawn_github(config: &crate::config::Config) -> std::sync::Arc<GithubHarness> {
    let harness = std::sync::Arc::new(GithubHarness::try_new(&config.github).await);
    super::helpers::warn_if_github_tools_missing(harness.as_ref()).await;
    harness
}
