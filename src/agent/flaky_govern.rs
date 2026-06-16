use chrono::Utc;
use tokio::sync::broadcast;

use crate::app::AppEvent;
use crate::config::Config;
use crate::engine::AgentSpec;
use crate::error::Result;
use crate::mcp::McpClient;
use crate::output::digest::{publish_digest, IncrementalDigest};
use crate::store::{FlakyQuery, Store};

pub struct FlakyGovernOutcome {
    pub test_count: u32,
}

impl FlakyGovernOutcome {
    pub fn format_summary(&self) -> String {
        format!(
            "flaky-govern: {} flaky test(s) in report",
            self.test_count
        )
    }
}

pub async fn run_flaky_govern(
    config: &Config,
    _mcp: &dyn McpClient,
    store: &dyn Store,
    agent: &AgentSpec,
    events: &broadcast::Sender<AppEvent>,
    log: impl Fn(&str, String),
) -> Result<FlakyGovernOutcome> {
    let tests = store
        .list_flaky_tests(FlakyQuery {
            repo: None,
            since_days: Some(30),
            limit: 20,
        })
        .await?;

    log(
        "info",
        format!("flaky-govern: {} test(s) in Top-20 rollup", tests.len()),
    );

    let mut digest = IncrementalDigest::begin(agent);
    digest.begin_repo("all repos");

    let mut body = String::from("## Top flaky tests (30d)\n\n");
    if tests.is_empty() {
        body.push_str("_No flaky incidents recorded yet._\n");
        digest.push_report_line("_No flaky incidents recorded yet._");
    } else {
        body.push_str("| Test | Repo | Workflow | Count | Rerun rate |\n");
        body.push_str("|------|------|----------|-------|------------|\n");
        for t in &tests {
            let name = t.test_name.as_deref().unwrap_or("(unknown test)");
            let rate = if t.rerun_attempts == 0 {
                "—".into()
            } else {
                format!(
                    "{:.0}%",
                    100.0 * f64::from(t.rerun_successes) / f64::from(t.rerun_attempts)
                )
            };
            body.push_str(&format!(
                "| {name} | {} | {} | {} | {rate} |\n",
                t.repo, t.workflow, t.incident_count
            ));
            digest.push_report_line(&format!(
                "- **{name}** ({}) — {}× in `{}`, rerun {rate}",
                t.repo, t.incident_count, t.workflow
            ));
        }
    }

    body.push_str(&format!(
        "\n_Report generated at {}._\n",
        Utc::now().format("%Y-%m-%d %H:%M UTC")
    ));

    let final_digest = digest.finish();
    publish_digest(config, store, events, &final_digest).await?;

    Ok(FlakyGovernOutcome {
        test_count: tests.len() as u32,
    })
}

pub fn format_flaky_report_csv(tests: &[crate::store::FlakyTestRollup]) -> String {
    let mut out = String::from("test,repo,workflow,incident_count,rerun_attempts,rerun_successes\n");
    for t in tests {
        let name = t.test_name.as_deref().unwrap_or("");
        out.push_str(&format!(
            "\"{}\",\"{}\",\"{}\",{},{},{}\n",
            name.replace('"', "\"\""),
            t.repo,
            t.workflow,
            t.incident_count,
            t.rerun_attempts,
            t.rerun_successes
        ));
    }
    out
}
