use chrono::Utc;
use tokio::sync::broadcast;

use crate::agent::parse::parse_pr_line;
use crate::app::AppEvent;
use crate::config::Config;
use crate::error::{CoworkerError, Result};
use crate::github::helpers::gh_tool;
use crate::github::GithubHarness;
use crate::output::digest::{publish_digest, IncrementalDigest};
use crate::store::{PrSnapshot, Store};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewBlockedPr {
    pub repo: String,
    pub number: u32,
    pub title: String,
    pub author: String,
}

impl ReviewBlockedPr {
    pub fn url(&self) -> String {
        format!("https://github.com/{}/pull/{}", self.repo, self.number)
    }

    pub fn one_liner(&self) -> String {
        format!(
            "{repo}#{number} {title} (@{author}) — {url}",
            repo = self.repo,
            number = self.number,
            title = self.title,
            author = self.author,
            url = self.url(),
        )
    }
}

pub struct ReviewRadarOutcome {
    pub prs: Vec<ReviewBlockedPr>,
}

impl ReviewRadarOutcome {
    pub fn format_summary(&self) -> String {
        if self.prs.is_empty() {
            return "review-radar: no PRs waiting for review".into();
        }
        let mut out = format!(
            "review-radar: {} PR(s) waiting for review:\n",
            self.prs.len()
        );
        for pr in &self.prs {
            out.push_str(&format!("  - {}\n", pr.one_liner()));
        }
        out.trim_end().to_string()
    }
}

pub async fn run_review_radar(
    config: &Config,
    github: &GithubHarness,
    store: &dyn Store,
    workflow_id: &str,
    events: &broadcast::Sender<AppEvent>,
    log: impl Fn(&str, String),
) -> Result<ReviewRadarOutcome> {
    if !github.is_available() {
        return Err(CoworkerError::Workflow(
            "GitHub harness (gh) is required for review-radar".into(),
        ));
    }

    let mut digest = IncrementalDigest::begin(workflow_id);
    publish_digest(config, store, events, &digest.to_digest()).await?;
    log(
        "info",
        "review-radar digest export started (in progress)".into(),
    );

    let mut prs = Vec::new();

    for repo in config.repos.clone() {
        log(
            "info",
            format!("review-radar: waiting-for-review PRs in {repo}"),
        );
        let list_text = gh_tool(
            github,
            "pr_list_waiting_review",
            serde_json::json!({
                "repo": repo,
                "limit": config.policy.max_prs_per_repo,
            }),
        )
        .await?;

        digest.begin_repo(&repo);

        for line in list_text.lines() {
            let Some(pr) = parse_pr_line(line) else {
                continue;
            };

            let blocked = ReviewBlockedPr {
                repo: repo.clone(),
                number: pr.number,
                title: pr.title.clone(),
                author: pr.author.clone(),
            };
            log(
                "info",
                format!("waiting for review: {}", blocked.one_liner()),
            );
            digest.push_waiting_review(&repo, pr.number, &pr.title, &pr.ci, Some(&pr.author));
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
                    triage_note: Some("review blocked".into()),
                })
                .await?;
            prs.push(blocked);
        }

        publish_digest(config, store, events, &digest.to_digest()).await?;
    }

    let final_digest = digest.finish();
    publish_digest(config, store, events, &final_digest).await?;

    Ok(ReviewRadarOutcome { prs })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_lists_prs() {
        let outcome = ReviewRadarOutcome {
            prs: vec![ReviewBlockedPr {
                repo: "acme/widget".into(),
                number: 19194,
                title: "docs: example".into(),
                author: "alice".into(),
            }],
        };
        let s = outcome.format_summary();
        assert!(s.contains("1 PR(s) waiting for review"));
        assert!(s.contains("acme/widget#19194"));
        assert!(s.contains("https://github.com/acme/widget/pull/19194"));
    }

    #[test]
    fn empty_summary() {
        assert_eq!(
            ReviewRadarOutcome { prs: vec![] }.format_summary(),
            "review-radar: no PRs waiting for review"
        );
    }
}
