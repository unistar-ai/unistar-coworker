use chrono::Utc;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::agent::parse::{
    github_actions_run_url, leading_failure_streak, parse_branch_runs, ParsedBranchRun,
};
use crate::app::AppEvent;
use crate::config::Config;
use crate::engine::AgentSpec;
use crate::error::{CoworkerError, Result};
use crate::mcp::helpers::lazy_tool;
use crate::mcp::McpClient;
use crate::output::digest::{publish_digest, IncrementalDigest};
use crate::store::{MainAlert, MainAlertQuery, Store};

pub struct MainGuardOutcome {
    pub alerts: Vec<MainAlert>,
}

impl MainGuardOutcome {
    pub fn format_summary(&self) -> String {
        if self.alerts.is_empty() {
            return "main-guard: all clear — no main branch alerts".into();
        }
        let mut out = format!("main-guard: {} alert(s):\n", self.alerts.len());
        for alert in &self.alerts {
            out.push_str(&format!(
                "  - {}@{} — {} consecutive failure(s); latest [{run}]({url}) {wf} ({concl})\n",
                alert.repo,
                alert.branch,
                alert.consecutive_failures,
                run = alert.latest_run_id,
                url = github_actions_run_url(&alert.repo, alert.latest_run_id),
                wf = alert.latest_workflow,
                concl = alert.conclusion,
            ));
        }
        out.trim_end().to_string()
    }
}

pub async fn run_main_guard(
    config: &Config,
    mcp: &dyn McpClient,
    store: &dyn Store,
    agent: &AgentSpec,
    events: &broadcast::Sender<AppEvent>,
    log: impl Fn(&str, String),
) -> Result<MainGuardOutcome> {
    if !mcp.is_available() {
        return Err(CoworkerError::Workflow(
            "unistar-mcp is required for main-guard".into(),
        ));
    }

    let threshold = config.main_guard.consecutive_failures.max(1);
    let limit = config.main_guard.recent_runs.clamp(5, 50);

    let mut digest = IncrementalDigest::begin(agent);
    publish_digest(config, store, events, &digest.to_digest()).await?;

    let mut alerts = Vec::new();

    for repo in config.repos.clone() {
        log("info", format!("checking default-branch CI for {repo}"));
        let text = lazy_tool(
            mcp,
            "ci_list_runs",
            serde_json::json!({
                "repo": repo,
                "limit": limit,
            }),
        )
        .await?;

        let (branch, runs) = parse_branch_runs(&text);
        let branch = branch.unwrap_or_else(|| "main".into());
        let streak = leading_failure_streak(&runs);
        log(
            "info",
            format!("{repo}@{branch}: {streak} consecutive failure(s) in recent runs"),
        );

        if streak < threshold {
            continue;
        }

        let Some(latest) = first_failed_run(&runs) else {
            continue;
        };

        if alert_already_recorded(store, &repo, latest.run_id).await? {
            log(
                "info",
                format!(
                    "main-guard: skip duplicate alert for {repo} run {}",
                    latest.run_id
                ),
            );
            continue;
        }

        let alert = MainAlert {
            id: Uuid::new_v4(),
            repo: repo.clone(),
            ts: Utc::now(),
            branch: branch.clone(),
            consecutive_failures: streak,
            latest_run_id: latest.run_id,
            latest_workflow: latest.workflow.clone(),
            conclusion: latest.conclusion.clone(),
            acknowledged: false,
        };
        store.record_main_alert(&alert).await?;
        log("warn", format!("main alert: {}", alert_line(&alert)));

        digest.begin_repo(&repo);
        digest.push_alert_line(&alert_line(&alert));
        alerts.push(alert);
    }

    let final_digest = digest.finish();
    if !alerts.is_empty() {
        publish_digest(config, store, events, &final_digest).await?;
    }

    Ok(MainGuardOutcome { alerts })
}

fn first_failed_run(runs: &[ParsedBranchRun]) -> Option<&ParsedBranchRun> {
    runs.iter().find(|r| {
        let c = r.conclusion.to_ascii_lowercase();
        !c.is_empty()
            && !matches!(c.as_str(), "in_progress" | "queued" | "waiting" | "pending")
            && crate::agent::parse::run_conclusion_is_failure(&c)
    })
}

fn alert_line(alert: &MainAlert) -> String {
    format!(
        "- [{repo}@{branch}](https://github.com/{repo}) — {count} consecutive failure(s); latest [{run}]({url}) {wf} ({concl})",
        repo = alert.repo,
        branch = alert.branch,
        count = alert.consecutive_failures,
        run = alert.latest_run_id,
        url = github_actions_run_url(&alert.repo, alert.latest_run_id),
        wf = alert.latest_workflow,
        concl = alert.conclusion,
    )
}

async fn alert_already_recorded(store: &dyn Store, repo: &str, run_id: i64) -> Result<bool> {
    let existing = store
        .list_main_alerts(MainAlertQuery {
            repo: Some(repo.to_string()),
            unacknowledged_only: true,
            since_hours: Some(24),
            limit: 20,
        })
        .await?;
    Ok(existing.iter().any(|a| a.latest_run_id == run_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_empty() {
        assert_eq!(
            MainGuardOutcome { alerts: vec![] }.format_summary(),
            "main-guard: all clear — no main branch alerts"
        );
    }
}
