use std::sync::Arc;

use tokio::sync::broadcast;
use uuid::Uuid;

use crate::agent::budget::TokenBudget;
use crate::app::{append_audit, AppEvent, SharedState};
use crate::config::Config;
use crate::engine::workflows::{load_skill, WorkflowRunner};
use crate::error::Result;
use crate::mcp::McpClient;
use crate::store::{compute_fingerprint, Classification, Digest, DigestSummary, FlakyIncident, Store};

use crate::output::export::maybe_export_digest;

pub struct AgentLoop {
    config: Config,
    store: Arc<dyn Store>,
    mcp: Arc<dyn McpClient>,
    events: broadcast::Sender<AppEvent>,
    state: SharedState,
}

impl AgentLoop {
    pub fn new(
        config: Config,
        store: Arc<dyn Store>,
        mcp: Arc<dyn McpClient>,
        events: broadcast::Sender<AppEvent>,
        state: SharedState,
    ) -> Self {
        Self {
            config,
            store,
            mcp,
            events,
            state,
        }
    }

    pub async fn run_workflow(&self, workflow_id: &str) -> Result<String> {
        let _budget = TokenBudget::from_config(self.config.llm.context_limit);
        let runner = WorkflowRunner::new(&self.config);
        let wf = runner.get(workflow_id)?;

        let _skill = load_skill(&wf.skill_path())?;
        let run_id = self.store.start_workflow_run(workflow_id).await?;
        let _ = self.events.send(AppEvent::WorkflowStarted {
            workflow_id: workflow_id.to_string(),
        });
        {
            let mut s = self.state.write().await;
            s.engine_busy = true;
            s.push_log("info", format!("workflow {workflow_id} started"));
        }

        let result = match workflow_id {
            "daily-work" => self.run_daily_work().await,
            other => {
                append_audit(
                    self.store.as_ref(),
                    "warn",
                    "workflow",
                    &format!("workflow {other} not implemented in v0.1"),
                )
                .await;
                Err(crate::error::CoworkerError::Workflow(format!(
                    "workflow {other} not implemented yet"
                )))
            }
        };

        match &result {
            Ok(summary) => {
                self.store
                    .finish_workflow_run(&run_id, Some(summary), None)
                    .await?;
                let _ = self.events.send(AppEvent::WorkflowFinished {
                    workflow_id: workflow_id.to_string(),
                    ok: true,
                    message: summary.clone(),
                });
            }
            Err(e) => {
                self.store
                    .finish_workflow_run(&run_id, None, Some(&e.to_string()))
                    .await?;
                let _ = self.events.send(AppEvent::WorkflowFinished {
                    workflow_id: workflow_id.to_string(),
                    ok: false,
                    message: e.to_string(),
                });
            }
        }

        {
            let mut s = self.state.write().await;
            s.engine_busy = false;
        }
        let _ = self.events.send(AppEvent::StoreUpdated);
        result
    }

    async fn run_daily_work(&self) -> Result<String> {
        let mut needs_attention = 0u32;
        let mut ignorable = 0u32;
        let mut flaky_candidates = 0u32;
        let mut body = String::from("# Daily Digest\n\n");

        for repo in &self.config.repos.clone() {
            body.push_str(&format!("## {repo}\n\n"));
            let list_result = self
                .mcp
                .tool_call(
                    "tool_call",
                    serde_json::json!({
                        "name": "pr_list_open",
                        "args": { "repo": repo, "limit": self.config.policy.max_prs_per_repo }
                    }),
                )
                .await;

            match list_result {
                Ok(text) => {
                    body.push_str(&text);
                    body.push_str("\n\n");
                    self.ingest_pr_lines(repo, &text).await?;
                }
                Err(e) => {
                    body.push_str(&format!("*MCP unavailable: {e}. Stub listing skipped.*\n\n"));
                    append_audit(
                        self.store.as_ref(),
                        "error",
                        "mcp",
                        &format!("pr_list_open failed for {repo}: {e}"),
                    )
                    .await;
                }
            }
        }

        for snap in self.store.list_pr_snapshots(None).await? {
            if snap.ci_summary.contains('✗') || snap.ci_summary.to_ascii_lowercase().contains("fail") {
                needs_attention += 1;
            } else if snap.is_draft {
                ignorable += 1;
            } else {
                ignorable += 1;
            }
        }

        flaky_candidates = self
            .store
            .list_flaky_tests(crate::store::FlakyQuery {
                repo: None,
                since_days: Some(1),
                limit: 100,
            })
            .await?
            .len() as u32;

        let digest = Digest {
            id: Uuid::new_v4(),
            date: chrono::Utc::now().date_naive(),
            summary: DigestSummary {
                needs_attention,
                ignorable,
                flaky_candidates,
            },
            body_md: body,
            created_at: chrono::Utc::now(),
        };

        self.store.save_digest(&digest).await?;
        maybe_export_digest(&self.config, &digest)?;
        let _ = self.events.send(AppEvent::DigestReady(digest));

        Ok(format!(
            "digest saved ({needs_attention} need attention, {flaky_candidates} flaky)"
        ))
    }

    async fn ingest_pr_lines(&self, repo: &str, text: &str) -> Result<()> {
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with("Open PRs") {
                continue;
            }
            // Expected compact line: "#123 title — CI … — review …"
            if let Some(rest) = line.strip_prefix('#') {
                let number_str: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
                if let Ok(number) = number_str.parse::<u32>() {
                    let snap = crate::store::PrSnapshot {
                        repo: repo.to_string(),
                        number,
                        title: line.to_string(),
                        author: String::new(),
                        ci_summary: line.to_string(),
                        review_summary: String::new(),
                        is_draft: line.to_ascii_lowercase().contains("draft"),
                        fetched_at: chrono::Utc::now(),
                        triage_note: None,
                    };
                    self.store.upsert_pr_snapshot(&snap).await?;
                }
            }
        }
        Ok(())
    }

    pub async fn record_stub_flaky(&self, repo: &str, pr: u32, run_id: i64) -> Result<()> {
        let error_sig = "timeout waiting for condition";
        let fp = compute_fingerprint(repo, "ci", Some("test"), None, error_sig);
        let incident = FlakyIncident {
            id: Uuid::new_v4(),
            ts: chrono::Utc::now(),
            repo: repo.to_string(),
            pr_number: Some(pr),
            run_id,
            workflow: "ci".into(),
            job: Some("test".into()),
            step: None,
            test_name: None,
            fingerprint: fp,
            classification: Classification::LlmFlaky,
            log_excerpt: error_sig.into(),
            llm_reason: Some("stub incident for demo".into()),
            rerun_outcome: None,
        };
        self.store.record_flaky_incident(&incident).await?;
        Ok(())
    }
}
