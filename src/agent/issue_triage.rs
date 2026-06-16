use chrono::Utc;
use tokio::sync::broadcast;

use crate::agent::parse::parse_issue_line;
use crate::app::AppEvent;
use crate::config::Config;
use crate::engine::AgentSpec;
use crate::error::{CoworkerError, Result};
use crate::mcp::helpers::lazy_tool;
use crate::mcp::McpClient;
use crate::output::digest::{publish_digest, IncrementalDigest};
use crate::store::{IssueSnapshot, Store};

pub struct IssueTriageOutcome {
    pub issues: u32,
}

impl IssueTriageOutcome {
    pub fn format_summary(&self) -> String {
        format!("issue-triage: {} open issue(s) indexed", self.issues)
    }
}

pub async fn run_issue_triage(
    config: &Config,
    mcp: &dyn McpClient,
    store: &dyn Store,
    agent: &AgentSpec,
    events: &broadcast::Sender<AppEvent>,
    log: impl Fn(&str, String),
) -> Result<IssueTriageOutcome> {
    if !mcp.is_available() {
        return Err(CoworkerError::Workflow(
            "unistar-mcp is required for issue-triage".into(),
        ));
    }

    let mut digest = IncrementalDigest::begin(agent);
    let mut total = 0u32;

    for repo in config.repos.clone() {
        log("info", format!("listing open issues for {repo}"));
        let list_text = lazy_tool(
            mcp,
            "issue_list_open",
            serde_json::json!({ "repo": repo, "limit": config.policy.max_prs_per_repo }),
        )
        .await?;

        digest.begin_repo(&repo);

        for line in list_text.lines() {
            let Some(iss) = parse_issue_line(line) else {
                continue;
            };
            total += 1;
            let note = if iss.labels == "(none)" {
                "untagged".into()
            } else {
                format!("labels: {}", iss.labels)
            };
            digest.push_report_line(&format!(
                "#{num} {title} (@{author}) — {note}",
                num = iss.number,
                title = iss.title,
                author = iss.author,
                note = note
            ));

            let updated_at = iss
                .updated
                .parse::<chrono::NaiveDate>()
                .map(|d| d.and_hms_opt(0, 0, 0).unwrap().and_utc())
                .unwrap_or_else(|_| Utc::now());

            store
                .upsert_issue_snapshot(&IssueSnapshot {
                    repo: repo.clone(),
                    number: iss.number,
                    title: iss.title.clone(),
                    author: iss.author.clone(),
                    labels: iss.labels.clone(),
                    updated_at,
                    fetched_at: Utc::now(),
                    triage_note: Some(note),
                })
                .await?;
        }

        if total == 0 {
            digest.push_report_line("_No open issues parsed._");
        }
    }

    let final_digest = digest.finish();
    publish_digest(config, store, events, &final_digest).await?;

    Ok(IssueTriageOutcome { issues: total })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_format() {
        let o = IssueTriageOutcome { issues: 3 };
        assert!(o.format_summary().contains("3"));
    }
}
