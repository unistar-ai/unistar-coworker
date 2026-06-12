use std::time::Instant;

use chrono::Utc;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::agent::triage::{TriageOutcome, TriageRunEntry};
use crate::app::AppEvent;
use crate::config::Config;
use crate::engine::Skill;
use crate::error::Result;
use crate::llm::ClassifyVerdict;
use crate::output::export::maybe_export_digest;
use crate::store::{Digest, DigestSummary, Store, format_duration};

/// Builds and publishes a digest incrementally during daily-work.
pub struct IncrementalDigest {
    id: Uuid,
    date: chrono::NaiveDate,
    started: Instant,
    skill_name: String,
    needs_attention: u32,
    ignorable: u32,
    flaky_candidates: u32,
    policy_gates: u32,
    attention_section: String,
    flaky_section: String,
    policy_section: String,
    ok_section: String,
    processed_prs: u32,
    complete: bool,
}

impl IncrementalDigest {
    pub fn begin(skill: &Skill) -> Self {
        let skill_name = if skill.name.is_empty() {
            "daily-work".into()
        } else {
            skill.name.clone()
        };
        Self {
            id: Uuid::new_v4(),
            date: Utc::now().date_naive(),
            started: Instant::now(),
            skill_name,
            needs_attention: 0,
            ignorable: 0,
            flaky_candidates: 0,
            policy_gates: 0,
            attention_section: String::from("## Needs attention\n\n"),
            flaky_section: String::from("## Flaky candidates\n\n"),
            policy_section: String::from("## Policy gates\n\n"),
            ok_section: String::from("## OK / ignorable\n\n"),
            processed_prs: 0,
            complete: false,
        }
    }

    pub fn begin_repo(&mut self, repo: &str) {
        self.attention_section
            .push_str(&format!("### {repo}\n\n"));
        self.flaky_section.push_str(&format!("### {repo}\n\n"));
        self.policy_section.push_str(&format!("### {repo}\n\n"));
        self.ok_section.push_str(&format!("### {repo}\n\n"));
    }

    pub fn push_draft(&mut self, number: u32, title: &str) {
        self.ignorable += 1;
        self.processed_prs += 1;
        self.ok_section
            .push_str(&format!("- #{number} {title} (draft)\n"));
    }

    /// Route triage runs into digest sections by LLM verdict (mixed PRs split per run).
    pub fn push_triage(&mut self, number: u32, title: &str, outcome: &TriageOutcome) {
        self.processed_prs += 1;

        if outcome.fallback_attention {
            self.needs_attention += 1;
            append_pr_block(
                &mut self.attention_section,
                number,
                title,
                Some("CI failure"),
                &outcome.preamble,
                &outcome.runs,
            );
            return;
        }

        let attention: Vec<TriageRunEntry> = outcome
            .runs
            .iter()
            .filter(|r| matches!(r.verdict, ClassifyVerdict::Real | ClassifyVerdict::Unknown))
            .cloned()
            .collect();
        let flaky: Vec<TriageRunEntry> = outcome
            .runs
            .iter()
            .filter(|r| r.verdict == ClassifyVerdict::Flaky)
            .cloned()
            .collect();
        let policy: Vec<TriageRunEntry> = outcome
            .runs
            .iter()
            .filter(|r| r.verdict == ClassifyVerdict::Policy)
            .cloned()
            .collect();

        let mut remaining = &outcome.preamble[..];

        if !attention.is_empty() {
            self.needs_attention += 1;
            let (head, tail) = split_preamble(remaining);
            append_pr_block(
                &mut self.attention_section,
                number,
                title,
                Some("CI failure"),
                head,
                &attention,
            );
            remaining = tail;
        }

        if !flaky.is_empty() {
            self.flaky_candidates += 1;
            let (head, tail) = split_preamble(remaining);
            append_pr_block(
                &mut self.flaky_section,
                number,
                title,
                Some("flaky"),
                head,
                &flaky,
            );
            remaining = tail;
        }

        if !policy.is_empty() {
            self.policy_gates += 1;
            append_policy_block(&mut self.policy_section, number, title, &policy);
        }

        let _ = remaining;
    }

    pub fn push_waiting_review(&mut self, number: u32, title: &str, ci: &str) {
        self.ignorable += 1;
        self.processed_prs += 1;
        self.ok_section.push_str(&format!(
            "- #{number} {title} — waiting for review (CI: {ci})\n"
        ));
    }

    pub fn push_ok(&mut self, number: u32, title: &str, ci: &str, review: &str) {
        self.ignorable += 1;
        self.processed_prs += 1;
        self.ok_section
            .push_str(&format!("- #{number} {title} CI:{ci} review:{review}\n"));
    }

    pub fn finish(mut self) -> Digest {
        self.complete = true;
        self.to_digest()
    }

    pub fn to_digest(&self) -> Digest {
        let duration_secs = self.started.elapsed().as_secs_f64();
        let duration_label = format_duration(duration_secs);
        let summary_counts = format!(
            "{} need attention, {} flaky, {} policy, {} ignorable",
            self.needs_attention, self.flaky_candidates, self.policy_gates, self.ignorable
        );
        let (status_block, summary_line) = if self.complete {
            (
                format!("Status: **complete**\nRun time: {duration_label}\n"),
                format!("Summary: {summary_counts}"),
            )
        } else {
            (
                format!(
                    "Status: **in progress** ({processed} PRs processed, run time so far: {duration_label})\n",
                    processed = self.processed_prs,
                ),
                format!("Summary so far: {summary_counts}"),
            )
        };

        let body_md = format!(
            "# Daily Digest\n\n\
Skill: {skill}\n\n\
{status_block}\
{summary_line}\n\n\
{attention}\n\
{flaky}\n\
{policy}\n\
{ok}",
            skill = self.skill_name,
            attention = self.attention_section,
            flaky = self.flaky_section,
            policy = self.policy_section,
            ok = self.ok_section,
        );

        Digest {
            id: self.id,
            date: self.date,
            summary: DigestSummary {
                needs_attention: self.needs_attention,
                ignorable: self.ignorable,
                flaky_candidates: self.flaky_candidates,
                policy_gates: self.policy_gates,
                duration_secs,
                complete: self.complete,
            },
            body_md,
            created_at: Utc::now(),
        }
    }
}

fn split_preamble(preamble: &[String]) -> (&[String], &[String]) {
    if preamble.is_empty() {
        (&[], &[])
    } else {
        (&preamble[..1], &preamble[1..])
    }
}

fn append_policy_block(section: &mut String, number: u32, title: &str, runs: &[TriageRunEntry]) {
    section.push_str(&format!("- #{number} {title}\n"));
    for run in runs {
        for line in &run.lines {
            section.push_str(&format!("{line}\n"));
        }
    }
}

fn append_pr_block(
    section: &mut String,
    number: u32,
    title: &str,
    label: Option<&str>,
    preamble: &[String],
    runs: &[TriageRunEntry],
) {
    if label.is_some() {
        section.push_str(&format!(
            "- #{number} {title} — {}\n",
            label.unwrap_or("CI")
        ));
    } else {
        section.push_str(&format!("- #{number} {title}\n"));
    }
    for line in preamble {
        section.push_str(&format!("  {line}\n"));
    }
    for run in runs {
        for line in &run.lines {
            section.push_str(&format!("  {line}\n"));
        }
    }
}

pub async fn publish_digest(
    config: &Config,
    store: &dyn Store,
    events: &broadcast::Sender<AppEvent>,
    digest: &Digest,
) -> Result<()> {
    store.save_digest(digest).await?;
    maybe_export_digest(config, digest)?;
    let _ = events.send(AppEvent::DigestReady(digest.clone()));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::triage::TriageRunEntry;
    use crate::engine::Skill;

    fn test_skill() -> Skill {
        Skill {
            name: "daily-work".into(),
            description: String::new(),
            body: String::new(),
        }
    }

    #[test]
    fn partial_digest_marks_in_progress() {
        let mut d = IncrementalDigest::begin(&test_skill());
        d.begin_repo("org/repo");
        d.push_ok(1, "fix", "pass", "approved");
        let digest = d.to_digest();
        assert!(!digest.summary.complete);
        assert!(digest.body_md.contains("in progress"));
    }

    #[test]
    fn mixed_pr_splits_runs_by_verdict() {
        let mut d = IncrementalDigest::begin(&test_skill());
        d.begin_repo("Kong/kong-ee");
        let outcome = TriageOutcome {
            preamble: vec!["PR #1 open".into()],
            runs: vec![
                TriageRunEntry {
                    verdict: ClassifyVerdict::Policy,
                    lines: vec!["  - [1](http://x) approval checker — obtain approval".into()],
                },
                TriageRunEntry {
                    verdict: ClassifyVerdict::Flaky,
                    lines: vec!["- run [2](http://x) build → flaky".into()],
                },
            ],
            fallback_attention: false,
        };
        d.push_triage(1, "backport", &outcome);
        let digest = d.finish();
        assert_eq!(digest.summary.policy_gates, 1);
        assert_eq!(digest.summary.flaky_candidates, 1);
        assert_eq!(digest.summary.needs_attention, 0);
        assert!(digest.body_md.contains("## Policy gates"));
        assert!(digest.body_md.contains("approval checker — obtain approval"));
        assert!(!digest.body_md.contains("Diagnosis:"));
        assert!(digest.body_md.contains("build → flaky"));
        assert!(!digest.body_md.contains("## Needs attention\n\n### Kong/kong-ee\n\n- #1"));
    }

    #[test]
    fn policy_only_skips_needs_attention() {
        let mut d = IncrementalDigest::begin(&test_skill());
        d.begin_repo("org/repo");
        let outcome = TriageOutcome {
            preamble: vec!["PR status".into()],
            runs: vec![TriageRunEntry {
                verdict: ClassifyVerdict::Policy,
                lines: vec!["  - [9](http://x) checker — obtain approval".into()],
            }],
            fallback_attention: false,
        };
        d.push_triage(9, "backport", &outcome);
        let digest = d.finish();
        assert_eq!(digest.summary.needs_attention, 0);
        assert_eq!(digest.summary.policy_gates, 1);
    }
}
