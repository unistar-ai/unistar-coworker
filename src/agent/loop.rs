use std::sync::Arc;

use tokio::sync::broadcast;
use uuid::Uuid;

use crate::agent::parse::{ci_is_failing, needs_review, parse_pr_line};
use crate::agent::release::run_release_duty;
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
use crate::output::export::maybe_export_digest;
use crate::store::{Digest, DigestSummary, PrSnapshot, Store};

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
        let runner = WorkflowRunner::new(&self.config);
        let wf = runner.get(workflow_id)?;
        let skill = load_skill(wf.skill_path())?;

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

        self.log(
            "info",
            format!(
                "loaded skill '{}' ({} chars)",
                skill.name,
                skill.body.len()
            ),
        );

        let mut needs_attention = 0u32;
        let mut ignorable = 0u32;
        let mut flaky_candidates = 0u32;

        let mut attention_section = String::from("## Needs attention\n\n");
        let mut flaky_section = String::from("## Flaky candidates\n\n");
        let mut ok_section = String::from("## OK / ignorable\n\n");

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

            attention_section.push_str(&format!("### {repo}\n\n"));
            flaky_section.push_str(&format!("### {repo}\n\n"));
            ok_section.push_str(&format!("### {repo}\n\n"));

            for line in list_text.lines() {
                let Some(pr) = parse_pr_line(line) else {
                    continue;
                };

                if pr.is_draft {
                    ignorable += 1;
                    ok_section.push_str(&format!(
                        "- #{} {} (draft)\n",
                        pr.number, pr.title
                    ));
                    self.save_pr_snapshot(&repo, &pr, None).await?;
                    continue;
                }

                let mut handled = false;

                if ci_is_failing(&pr.ci) {
                    self.log("info", format!("triaging {repo}#{}", pr.number));
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

                    if outcome.flaky {
                        flaky_candidates += 1;
                        flaky_section.push_str(&format!(
                            "- #{} {} — flaky\n  {}\n",
                            pr.number,
                            pr.title,
                            outcome.note.replace('\n', "\n  ")
                        ));
                    }
                    if outcome.real {
                        needs_attention += 1;
                        attention_section.push_str(&format!(
                            "- #{} {} — CI failure\n  {}\n",
                            pr.number,
                            pr.title,
                            outcome.note.replace('\n', "\n  ")
                        ));
                    }
                    if !outcome.flaky && !outcome.real {
                        needs_attention += 1;
                        attention_section.push_str(&format!(
                            "- #{} {} — CI unclear\n  {}\n",
                            pr.number,
                            pr.title,
                            outcome.note.replace('\n', "\n  ")
                        ));
                    }
                    handled = true;
                } else if needs_review(&pr.review) && pr.review != "approved" {
                    needs_attention += 1;
                    attention_section.push_str(&format!(
                        "- #{} {} — waiting for review (CI: {})\n",
                        pr.number, pr.title, pr.ci
                    ));
                    self.save_pr_snapshot(&repo, &pr, Some("review blocked".into()))
                        .await?;
                    handled = true;
                }

                if !handled {
                    ignorable += 1;
                    ok_section.push_str(&format!(
                        "- #{} {} CI:{} review:{}\n",
                        pr.number, pr.title, pr.ci, pr.review
                    ));
                    self.save_pr_snapshot(&repo, &pr, None).await?;
                }
            }
        }

        let body = format!(
            "# Daily Digest\n\n\
Skill: {}\n\n\
Summary: {} need attention, {} flaky, {} ignorable\n\n\
{attention_section}\n\
{flaky_section}\n\
{ok_section}"
        ,
            if skill.name.is_empty() {
                "daily-work"
            } else {
                &skill.name
            },
            needs_attention,
            flaky_candidates,
            ignorable,
        );

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

        append_audit(
            self.store.as_ref(),
            "info",
            "daily-work",
            &format!("digest: {needs_attention} attention, {flaky_candidates} flaky"),
        )
        .await;

        Ok(format!(
            "digest saved ({needs_attention} need attention, {flaky_candidates} flaky, {ignorable} ok)"
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

        Ok(format!(
            "release-duty: {} backport(s) queued, {} skipped",
            outcome.queued, outcome.skipped
        ))
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
