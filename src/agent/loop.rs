use std::sync::Arc;

use tokio::sync::broadcast;

use crate::agent::budget::TokenBudget;
use crate::agent::parse::{ci_is_failing, needs_review, parse_pr_line};
use crate::agent::review_radar::run_review_radar;
use crate::agent::triage::triage_pr;
use crate::app::{append_audit, AppEvent, SharedState};
use crate::config::Config;
use crate::engine::workflows::WorkflowRunner;
use crate::engine::{load_workflow_spec, WorkflowSpec};
use crate::error::{CoworkerError, Result};
use crate::github::helpers::gh_tool;
use crate::github::GithubHarness;
use crate::llm::LlmClient;
use crate::output::digest::{publish_digest, IncrementalDigest};
use crate::store::LogLine;
use crate::store::{PrSnapshot, Store};

pub struct AgentLoop {
    config: Config,
    store: Arc<dyn Store>,
    github: Arc<GithubHarness>,
    llm: Arc<LlmClient>,
    events: broadcast::Sender<AppEvent>,
    state: SharedState,
}

impl AgentLoop {
    pub fn new(
        config: Config,
        store: Arc<dyn Store>,
        github: Arc<GithubHarness>,
        llm: Arc<LlmClient>,
        events: broadcast::Sender<AppEvent>,
        state: SharedState,
    ) -> Self {
        Self {
            config,
            store,
            github,
            llm,
            events,
            state,
        }
    }

    fn log(&self, level: &str, message: impl Into<String>) {
        let msg = message.into();
        let _ = self.events.send(AppEvent::LogLine(LogLine {
            ts: chrono::Utc::now(),
            level: level.into(),
            message: msg.clone(),
        }));
        {
            let state = self.state.clone();
            let level = level.to_string();
            tokio::spawn(async move {
                let mut s = state.write().await;
                s.push_log(&level, msg);
            });
        }
    }

    pub async fn run_workflow(&self, workflow_id: &str) -> Result<String> {
        let budget = TokenBudget::from_config(self.config.llm.context_limit);
        let runner = WorkflowRunner::new(&self.config);
        let wf = runner.get(workflow_id)?;
        let spec = load_workflow_spec(workflow_id, wf.skill_paths())?;

        self.log(
            "info",
            format!(
                "workflow {} — {} skill(s), budget {} tokens{}",
                wf.id,
                spec.skills.len(),
                budget.input_budget(),
                wf.schedule
                    .as_ref()
                    .map(|s| format!(", cron {s}"))
                    .unwrap_or_default()
            ),
        );
        if !spec.description.is_empty() {
            self.log("info", spec.description.clone());
        }
        self.log(
            "info",
            format!(
                "llm: {} ({})",
                self.config.llm.model,
                if self.llm.is_online() {
                    "online"
                } else {
                    "offline/heuristic"
                }
            ),
        );

        let run_id = self.store.start_workflow_run(workflow_id).await?;
        let _ = self.events.send(AppEvent::WorkflowStarted {
            workflow_id: workflow_id.to_string(),
        });
        {
            let mut s = self.state.write().await;
            s.engine_busy = true;
            s.push_log("info", format!("workflow {workflow_id} started"));
        }

        if !self.github.is_available() {
            self.log(
                "error",
                "GitHub harness unavailable — set github.gh_command and GH_TOKEN",
            );
        }

        let result = match workflow_id {
            "daily-work" => self.run_daily_work(&spec).await,
            "review-radar" => self.run_review_radar(workflow_id).await,
            other => Err(CoworkerError::Workflow(format!(
                "unknown workflow: {other} (built-in: daily-work, review-radar)"
            ))),
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

    async fn run_daily_work(&self, spec: &WorkflowSpec) -> Result<String> {
        if !self.github.is_available() {
            return Err(CoworkerError::Workflow(
                "GitHub harness is required for daily-work".into(),
            ));
        }

        let mut digest = IncrementalDigest::begin(&spec.id);
        publish_digest(
            &self.config,
            self.store.as_ref(),
            &self.events,
            &digest.to_digest(),
        )
        .await?;
        self.log("info", "digest export started (in progress)");

        for repo in self.config.repos.clone() {
            self.log("info", format!("listing open PRs for {repo}"));
            let list_text = gh_tool(
                self.github.as_ref(),
                "pr_list_open",
                serde_json::json!({
                    "repo": repo,
                    "limit": self.config.policy.max_prs_per_repo,
                }),
            )
            .await?;

            digest.begin_repo(&repo);
            publish_digest(
                &self.config,
                self.store.as_ref(),
                &self.events,
                &digest.to_digest(),
            )
            .await?;

            for line in list_text.lines() {
                let Some(pr) = parse_pr_line(line) else {
                    continue;
                };

                if pr.is_draft {
                    digest.push_draft(pr.number, &pr.title);
                    self.save_pr_snapshot(&repo, &pr, None).await?;
                    publish_digest(
                        &self.config,
                        self.store.as_ref(),
                        &self.events,
                        &digest.to_digest(),
                    )
                    .await?;
                    continue;
                }

                let mut handled = false;

                if ci_is_failing(&pr.ci) {
                    self.log(
                        "info",
                        format!("triaging {repo}#{} (CI: {})", pr.number, pr.ci),
                    );
                    let outcome = triage_pr(
                        &self.config,
                        self.github.as_ref(),
                        self.llm.as_ref(),
                        self.store.as_ref(),
                        &spec.skills,
                        &repo,
                        &pr,
                        Some(&self.events),
                    )
                    .await?;

                    digest.push_triage(pr.number, &pr.title, &outcome);
                    handled = true;
                } else if needs_review(&pr.review) && pr.review != "approved" {
                    digest.push_waiting_review(
                        &repo,
                        pr.number,
                        &pr.title,
                        &pr.ci,
                        Some(&pr.author),
                    );
                    self.save_pr_snapshot(&repo, &pr, Some("review blocked".into()))
                        .await?;
                    handled = true;
                }

                if !handled {
                    digest.push_ok(pr.number, &pr.title, &pr.ci, &pr.review);
                    self.save_pr_snapshot(&repo, &pr, None).await?;
                }

                publish_digest(
                    &self.config,
                    self.store.as_ref(),
                    &self.events,
                    &digest.to_digest(),
                )
                .await?;
            }
        }

        let final_digest = digest.finish();
        let needs_attention = final_digest.summary.needs_attention;
        let ignorable = final_digest.summary.ignorable;
        let flaky_candidates = final_digest.summary.flaky_candidates;
        let policy_gates = final_digest.summary.policy_gates;
        let duration_label = final_digest.summary.duration_label();

        publish_digest(
            &self.config,
            self.store.as_ref(),
            &self.events,
            &final_digest,
        )
        .await?;

        append_audit(
            self.store.as_ref(),
            "info",
            "daily-work",
            &format!("digest: {needs_attention} attention, {flaky_candidates} flaky, {policy_gates} policy"),
        )
        .await;

        Ok(format!(
            "digest saved ({needs_attention} attention, {flaky_candidates} flaky, {policy_gates} policy, {ignorable} ok) in {duration_label}"
        ))
    }

    async fn run_review_radar(&self, workflow_id: &str) -> Result<String> {
        let outcome = run_review_radar(
            &self.config,
            self.github.as_ref(),
            self.store.as_ref(),
            workflow_id,
            &self.events,
            |level, msg| self.log(level, msg),
        )
        .await?;

        let summary = outcome.format_summary();
        self.log("info", summary.clone());

        append_audit(self.store.as_ref(), "info", "review-radar", &summary).await;

        Ok(summary)
    }

    async fn save_pr_snapshot(
        &self,
        repo: &str,
        pr: &crate::agent::parse::ParsedPrLine,
        triage_note: Option<String>,
    ) -> Result<()> {
        self.store
            .upsert_pr_snapshot(&PrSnapshot {
                repo: repo.to_string(),
                number: pr.number,
                title: pr.title.clone(),
                author: pr.author.clone(),
                ci_summary: pr.ci.clone(),
                review_summary: pr.review.clone(),
                is_draft: pr.is_draft,
                fetched_at: chrono::Utc::now(),
                triage_note,
            })
            .await
    }
}
