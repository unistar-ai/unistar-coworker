use chrono::Utc;
use tokio::sync::broadcast;

use crate::agent::parse::{categorize_my_pr, github_pr_url, parse_pr_line, MyPrCategory};
use crate::app::AppEvent;
use crate::config::Config;
use crate::engine::AgentSpec;
use crate::error::{CoworkerError, Result};
use crate::mcp::helpers::lazy_tool;
use crate::mcp::McpClient;
use crate::output::digest::{publish_digest, IncrementalDigest};
use crate::store::{PrSnapshot, Store};

pub struct MyPrBriefOutcome {
    pub total: u32,
    pub failing: u32,
    pub waiting: u32,
    pub ready: u32,
}

impl MyPrBriefOutcome {
    pub fn format_summary(&self) -> String {
        format!(
            "my-pr-brief: {} PR(s) — {} failing, {} waiting review, {} ready",
            self.total, self.failing, self.waiting, self.ready
        )
    }
}

pub async fn run_my_pr_brief(
    config: &Config,
    mcp: &dyn McpClient,
    store: &dyn Store,
    agent: &AgentSpec,
    events: &broadcast::Sender<AppEvent>,
    log: impl Fn(&str, String),
) -> Result<MyPrBriefOutcome> {
    if !mcp.is_available() {
        return Err(CoworkerError::Workflow(
            "unistar-mcp is required for my-pr-brief".into(),
        ));
    }

    let mut digest = IncrementalDigest::begin(agent);
    publish_digest(config, store, events, &digest.to_digest()).await?;

    let mut total = 0u32;
    let mut failing = 0u32;
    let mut waiting = 0u32;
    let mut ready = 0u32;

    for repo in config.repos.clone() {
        log("info", format!("listing my open PRs for {repo}"));
        let list_text = lazy_tool(
            mcp,
            "pr_list_open",
            serde_json::json!({
                "repo": repo,
                "author": "@me",
                "limit": config.policy.max_prs_per_repo,
            }),
        )
        .await?;

        digest.begin_repo(&repo);
        let mut repo_any = false;

        for line in list_text.lines() {
            let Some(pr) = parse_pr_line(line) else {
                continue;
            };
            repo_any = true;
            total += 1;

            let note = match categorize_my_pr(&pr) {
                MyPrCategory::Draft => continue,
                MyPrCategory::CiFailing => {
                    failing += 1;
                    let line = format!(
                        "- [#{n} {title}]({url}) (@{author}) CI:{ci} review:{review} — **CI failing**",
                        n = pr.number,
                        title = pr.title,
                        url = github_pr_url(&repo, pr.number),
                        author = pr.author,
                        ci = pr.ci,
                        review = pr.review,
                    );
                    digest.push_alert_line(&line);
                    Some("my pr: CI failing".into())
                }
                MyPrCategory::WaitingReview => {
                    waiting += 1;
                    digest.push_waiting_review(&repo, pr.number, &pr.title, &pr.ci, Some(&pr.author));
                    Some("my pr: waiting review".into())
                }
                MyPrCategory::Ready => {
                    ready += 1;
                    let line = format!(
                        "- [#{n} {title}]({url}) (@{author}) CI:{ci} review:{review} — **ready**",
                        n = pr.number,
                        title = pr.title,
                        url = github_pr_url(&repo, pr.number),
                        author = pr.author,
                        ci = pr.ci,
                        review = pr.review,
                    );
                    digest.push_ready_line(&line);
                    Some("my pr: ready".into())
                }
            };

            store
                .upsert_pr_snapshot(&PrSnapshot {
                    repo: repo.clone(),
                    number: pr.number,
                    title: pr.title.clone(),
                    author: pr.author.clone(),
                    ci_summary: pr.ci.clone(),
                    review_summary: pr.review.clone(),
                    is_draft: pr.is_draft,
                    fetched_at: Utc::now(),
                    triage_note: note,
                })
                .await?;
        }

        if !repo_any {
            digest.push_report_line("_No open PRs by @me._");
        }
    }

    let final_digest = digest.finish();
    publish_digest(config, store, events, &final_digest).await?;

    Ok(MyPrBriefOutcome {
        total,
        failing,
        waiting,
        ready,
    })
}
