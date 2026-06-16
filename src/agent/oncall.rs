use chrono::Utc;
use tokio::sync::broadcast;

use crate::agent::parse::github_actions_run_url;
use crate::app::AppEvent;
use crate::config::Config;
use crate::engine::AgentSpec;
use crate::error::Result;
use crate::mcp::McpClient;
use crate::store::{FlakyQuery, MainAlertQuery, Store};

pub struct OncallOutcome;

impl OncallOutcome {
    pub fn format_summary(&self) -> String {
        "oncall-handoff: handoff pack exported".into()
    }
}

pub async fn build_handoff_markdown(store: &dyn Store) -> Result<String> {
    let digest = store.latest_digest().await?;
    let approvals = store.list_pending_approvals().await?;
    let flaky = store
        .list_flaky_tests(FlakyQuery {
            repo: None,
            since_days: Some(1),
            limit: 10,
        })
        .await?;
    let main_alerts = store
        .list_main_alerts(MainAlertQuery {
            repo: None,
            unacknowledged_only: true,
            since_hours: Some(24),
            limit: 20,
        })
        .await?;

    let mut body = String::from("# On-call handoff\n\n");
    body.push_str(&format!(
        "Generated: {}\n\n",
        Utc::now().format("%Y-%m-%d %H:%M UTC")
    ));

    body.push_str("## Latest digest\n\n");
    match digest {
        Some(d) => {
            body.push_str(&format!(
                "- Date: {}\n- Summary: {} need attention, {} flaky, {} policy, {} ignorable\n",
                d.date,
                d.summary.needs_attention,
                d.summary.flaky_candidates,
                d.summary.policy_gates,
                d.summary.ignorable,
            ));
        }
        None => body.push_str("_No digest yet — run daily-work._\n"),
    }

    body.push_str("\n## Pending approvals\n\n");
    if approvals.is_empty() {
        body.push_str("_None._\n");
    } else {
        for a in &approvals {
            body.push_str(&format!("- {:?}: {}\n", a.kind, a.description));
        }
    }

    body.push_str("\n## Main alerts (24h, unack)\n\n");
    if main_alerts.is_empty() {
        body.push_str("_None._\n");
    } else {
        for a in &main_alerts {
            body.push_str(&format!(
                "- {}@{} — {} failure(s); [{run}]({url}) {wf}\n",
                a.repo,
                a.branch,
                a.consecutive_failures,
                run = a.latest_run_id,
                url = github_actions_run_url(&a.repo, a.latest_run_id),
                wf = a.latest_workflow,
            ));
        }
    }

    body.push_str("\n## Flaky tests (24h)\n\n");
    if flaky.is_empty() {
        body.push_str("_None._\n");
    } else {
        for t in &flaky {
            let name = t.test_name.as_deref().unwrap_or(&t.workflow);
            body.push_str(&format!("- {name} — {}× in {}\n", t.incident_count, t.repo));
        }
    }

    let links = store.list_regression_links(5).await?;
    body.push_str("\n## Regression hints (recent)\n\n");
    if links.is_empty() {
        body.push_str("_None._\n");
    } else {
        for link in &links {
            body.push_str(&format!(
                "- {} — {}\n",
                link.test_name.as_deref().unwrap_or(&link.repo),
                link.summary.lines().next().unwrap_or("(see store)")
            ));
        }
    }

    Ok(body)
}

pub async fn run_oncall_handoff(
    config: &Config,
    _mcp: &dyn McpClient,
    store: &dyn Store,
    agent: &AgentSpec,
    events: &broadcast::Sender<AppEvent>,
    log: impl Fn(&str, String),
) -> Result<OncallOutcome> {
    log("info", "oncall-handoff: aggregating store snapshot".into());

    let body = build_handoff_markdown(store).await?;

    let mut inc_digest = crate::output::digest::IncrementalDigest::begin(agent);
    inc_digest.begin_repo("handoff");
    for line in body.lines().skip(2) {
        if line.is_empty() {
            continue;
        }
        inc_digest.push_report_line(line);
    }
    let final_digest = inc_digest.finish();
    crate::output::digest::publish_digest(config, store, events, &final_digest).await?;

    Ok(OncallOutcome)
}
