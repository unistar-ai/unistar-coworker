use std::sync::Arc;

use tokio::sync::broadcast;

use crate::agent::budget::TokenBudget;
use crate::agent::parse::{ci_is_failing, needs_review, parse_pr_line};
use crate::agent::release::run_release_duty;
use crate::agent::review_radar::run_review_radar;
use crate::agent::triage::triage_pr;
use crate::app::{append_audit, AppEvent, SharedState};
use crate::store::LogLine;
use crate::config::Config;
use crate::engine::{load_skill, Skill};
use crate::engine::workflows::WorkflowRunner;
use crate::error::Result;
use crate::llm::LlmClient;
use crate::mcp::helpers::lazy_tool;
use crate::mcp::McpClient;
use crate::output::digest::{publish_digest, IncrementalDigest};
use crate::store::{PrSnapshot, Store};

pub struct AgentLoop {
    config: Config,
    store: Arc<dyn Store>,
    mcp: Arc<dyn McpClient>,
    llm: Arc<LlmClient>,
    events: broadcast::Sender<AppEvent>,
    state: SharedState,
}

impl AgentLoop {
    pub fn new(
        config: Config,
        store: Arc<dyn Store>,
        mcp: Arc<dyn McpClient>,
        llm: Arc<LlmClient>,
        events: broadcast::Sender<AppEvent>,
        state: SharedState,
    ) -> Self {
        Self {
            config,
            store,
            mcp,
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
        let skill = load_skill(wf.skill_path())?;

        self.log(
            "info",
            format!(
                "workflow {} — skill '{}' ({} chars, budget {} tokens){}",
                wf.id,
                skill.name,
                skill.body.len(),
                budget.input_budget(),
                wf.schedule
                    .as_ref()
                    .map(|s| format!(", cron {s}"))
                    .unwrap_or_default()
            ),
        );
        if !skill.description.is_empty() {
            self.log("info", skill.description.clone());
        }
        self.log(
            "info",
            format!(
                "llm: {} ({})",
                self.config.llm.model,
                if self.llm.is_online() { "online" } else { "offline/heuristic" }
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

        if !self.mcp.is_available() {
            self.log("error", "unistar-mcp unavailable — set mcp.command and GH_TOKEN");
        }

        let result = match workflow_id {
            "daily-work" => self.run_daily_work(&skill).await,
            "release-duty" => self.run_release_duty(&skill).await,
            "review-radar" => self.run_review_radar(&skill).await,
            other => {
                append_audit(
                    self.store.as_ref(),
                    "warn",
                    "workflow",
                    &format!("workflow {other} not implemented yet"),
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

    async fn run_daily_work(&self, skill: &Skill) -> Result<String> {
        if !self.mcp.is_available() {
            return Err(crate::error::CoworkerError::Workflow(
                "unistar-mcp is required for daily-work".into(),
            ));
        }

        let mut digest = IncrementalDigest::begin(skill);
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
            let list_text = lazy_tool(
                self.mcp.as_ref(),
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
                    self.log("info", format!("triaging {repo}#{} (CI: {})", pr.number, pr.ci));
                    let outcome = triage_pr(
                        &self.config,
                        self.mcp.as_ref(),
                        self.llm.as_ref(),
                        self.store.as_ref(),
                        skill,
                        &repo,
                        &pr,
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

    async fn run_release_duty(&self, skill: &Skill) -> Result<String> {
        if !self.mcp.is_available() {
            return Err(crate::error::CoworkerError::Workflow(
                "unistar-mcp is required for release-duty".into(),
            ));
        }

        let outcome = run_release_duty(
            &self.config,
            self.mcp.as_ref(),
            self.store.as_ref(),
            skill,
            |level, msg| self.log(level, msg),
        )
        .await?;

        self.log(
            "info",
            format!(
                "release-duty: {} queued, {} skipped",
                outcome.queued, outcome.skipped
            ),
        );

        append_audit(
            self.store.as_ref(),
            "info",
            "release-duty",
            &outcome.body_md,
        )
        .await;

        Ok(format!(
            "release-duty: {} backport(s) queued, {} skipped",
            outcome.queued, outcome.skipped
        ))
    }

    async fn run_review_radar(&self, skill: &Skill) -> Result<String> {
        let outcome = run_review_radar(
            &self.config,
            self.mcp.as_ref(),
            self.store.as_ref(),
            skill,
            &self.events,
            |level, msg| self.log(level, msg),
        )
        .await?;

        let summary = outcome.format_summary();
        self.log("info", summary.clone());

        append_audit(
            self.store.as_ref(),
            "info",
            "review-radar",
            &summary,
        )
        .await;

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
