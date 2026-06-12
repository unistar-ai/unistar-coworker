use serde::{Deserialize, Serialize};

use crate::config::LlmConfig;
use crate::error::{CoworkerError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClassifyVerdict {
    Flaky,
    Real,
    /// Label, approval, template, or other repo policy — not a test flake.
    Policy,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ClassifyResponse {
    verdict: ClassifyVerdict,
    reason: String,
    #[serde(default)]
    test_name: Option<String>,
    /// Short summary of this page for the next page (when verdict is unknown).
    #[serde(default)]
    page_summary: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ClassifyResult {
    pub verdict: ClassifyVerdict,
    pub reason: String,
    pub test_name: Option<String>,
    pub used_llm: bool,
    pub pages_read: u32,
    /// LLM-provided note for the next page when verdict is unknown.
    pub page_summary: Option<String>,
}

pub struct LlmClient {
    cfg: LlmConfig,
    http: reqwest::Client,
    online: bool,
}

impl LlmClient {
    pub fn new(cfg: LlmConfig, online: bool) -> Self {
        Self {
            cfg,
            online,
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .expect("reqwest client"),
        }
    }

    pub fn is_online(&self) -> bool {
        self.online
    }

    /// Classify one page of CI logs. Each LLM call only sees this page plus `prior_summary`.
    #[allow(clippy::too_many_arguments)]
    pub async fn classify_log_page(
        &self,
        skill_body: &str,
        repo: &str,
        pr_number: u32,
        workflow: &str,
        page_logs: &str,
        combined_logs: &str,
        prior_summary: &str,
        page_num: u32,
        max_pages: u32,
    ) -> Result<ClassifyResult> {
        if let Some(result) = quick_classify(workflow, page_logs) {
            if result.verdict != ClassifyVerdict::Unknown {
                return Ok(ClassifyResult {
                    pages_read: page_num,
                    ..result
                });
            }
        }

        if self.online {
            match self
                .classify_with_llm(
                    skill_body,
                    repo,
                    pr_number,
                    workflow,
                    page_logs,
                    prior_summary,
                    page_num,
                    max_pages,
                )
                .await
            {
                Ok(r) => return Ok(r),
                Err(e) => {
                    tracing::warn!("LLM classify page {page_num} failed, using heuristics: {e}");
                }
            }
        }

        Ok(heuristic_classify(workflow, combined_logs))
    }

    #[allow(clippy::too_many_arguments)]
    async fn classify_with_llm(
        &self,
        skill_body: &str,
        repo: &str,
        pr_number: u32,
        workflow: &str,
        logs: &str,
        prior_summary: &str,
        page_num: u32,
        max_pages: u32,
    ) -> Result<ClassifyResult> {
        let system = format!(
            "{skill_body}\n\n\
You classify CI failures as flaky, real bugs, or policy/check gates. \
You may receive one page of logs at a time; prior pages are summarized, not repeated. \
If this page alone is inconclusive, respond verdict unknown and include page_summary \
(a ≤2 sentence note for the next page). \
Respond with JSON only, no markdown: \
{{\"verdict\":\"flaky\"|\"real\"|\"policy\"|\"unknown\",\"reason\":\"...\",\"test_name\":\"optional\",\"page_summary\":\"optional\"}}"
        );
        let prior = if prior_summary.is_empty() {
            "(none)".into()
        } else {
            prior_summary.to_string()
        };
        let user = format!(
            "repo: {repo}\npr: #{pr_number}\nworkflow: {workflow}\n\
log_page: {page_num}/{max_pages}\nprior_pages_summary: {prior}\n\n\
Failed logs (this page only):\n{logs}"
        );

        let url = format!(
            "{}/chat/completions",
            self.cfg.base_url.trim_end_matches('/')
        );
        let body = serde_json::json!({
            "model": self.cfg.model,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": user},
            ],
            "stream": false,
            "format": "json",
        });

        let resp = self
            .http
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| CoworkerError::Other(anyhow::anyhow!("llm request: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(CoworkerError::Other(anyhow::anyhow!(
                "llm HTTP {status}: {text}"
            )));
        }

        let v: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| CoworkerError::Other(anyhow::anyhow!("llm json: {e}")))?;

        let content = v
            .pointer("/choices/0/message/content")
            .and_then(|c| c.as_str())
            .ok_or_else(|| CoworkerError::Other(anyhow::anyhow!("llm missing content")))?;

        let parsed: ClassifyResponse = serde_json::from_str(content).map_err(|e| {
            CoworkerError::Other(anyhow::anyhow!("llm parse classify json: {e}; raw={content}"))
        })?;

        Ok(ClassifyResult {
            verdict: parsed.verdict,
            reason: parsed.reason,
            test_name: parsed.test_name,
            used_llm: true,
            pages_read: page_num,
            page_summary: parsed.page_summary,
        })
    }
}

pub fn append_log_chunk(combined: &mut String, chunk: &str) {
    if !combined.is_empty() {
        combined.push('\n');
    }
    combined.push_str(chunk);
    const MAX_COMBINED: usize = 24_000;
    if combined.len() > MAX_COMBINED {
        let keep = &combined[combined.len() - MAX_COMBINED..];
        *combined = format!("…[earlier log pages truncated]…\n{keep}");
    }
}

/// Rolling summary across pages — bounded so later LLM calls stay within context.
pub fn next_prior_summary(prior: &str, page_num: u32, result: &ClassifyResult) -> String {
    let page_note = result
        .page_summary
        .as_deref()
        .unwrap_or(&result.reason)
        .chars()
        .take(240)
        .collect::<String>();
    let mut next = if prior.is_empty() {
        format!("Page {page_num}: {page_note}")
    } else {
        format!("{prior} | Page {page_num}: {page_note}")
    };
    const MAX: usize = 800;
    if next.len() > MAX {
        next = next.chars().take(MAX).collect();
        next.push('…');
    }
    next
}

/// Workflow names that gate merges via labels, approvals, or templates — not test flakes.
pub fn is_policy_workflow(workflow: &str) -> bool {
    let w = workflow.to_ascii_lowercase();
    const NEEDLES: &[&str] = &[
        "approval checker",
        "backport pr",
        "check pr label",
        "pull request template",
        "template validation",
        "protected steps",
        "enforce protected",
        "changelog requirement",
        "required label",
        "label check",
        "pr label",
        "merge gate",
        "codecov",
        "danger",
        "semantic pull",
        "commitlint",
        "lint pr title",
    ];
    NEEDLES.iter().any(|n| w.contains(n))
}

fn policy_classify(workflow: &str) -> ClassifyResult {
    ClassifyResult {
        verdict: ClassifyVerdict::Policy,
        reason: format!(
            "policy check ({workflow}) — labels, approval, or template; not a flaky test"
        ),
        test_name: None,
        used_llm: false,
        pages_read: 1,
        page_summary: None,
    }
}

/// Fast path before LLM: workflow name or log shape.
pub fn quick_classify(workflow: &str, logs: &str) -> Option<ClassifyResult> {
    if is_policy_workflow(workflow) {
        return Some(policy_classify(workflow));
    }
    let lower = logs.to_ascii_lowercase();
    const POLICY_LOG: &[&str] = &[
        "required label",
        "missing label",
        "label is required",
        "does not have label",
        "approval required",
        "waiting for approval",
        "template validation",
        "pull_request_template",
        "changelog",
        "missing section",
        "not allowed to merge",
        "merge requirements",
    ];
    if POLICY_LOG.iter().any(|s| lower.contains(s)) {
        return Some(ClassifyResult {
            verdict: ClassifyVerdict::Policy,
            reason: "heuristic: policy/label/template signals in logs".into(),
            test_name: None,
            used_llm: false,
            pages_read: 1,
            page_summary: None,
        });
    }
    None
}

pub fn heuristic_classify(workflow: &str, logs: &str) -> ClassifyResult {
    if let Some(r) = quick_classify(workflow, logs) {
        return r;
    }

    let lower = logs.to_ascii_lowercase();
    let flaky_signals = [
        "timeout",
        "etimedout",
        "timed out",
        "connection reset",
        "connection refused",
        "network",
        "econnreset",
        "temporarily unavailable",
        "503",
        "502",
        "flake",
        "retry",
    ];
    let real_signals = [
        "panic:",
        "assertion",
        "assert_eq",
        "expected",
        "compile error",
        "error[E",
        "syntax error",
        "cannot find",
        "undefined reference",
        "test failed",
        "failures:",
        "exit code 1",
        "process completed with exit code",
    ];

    let flaky = flaky_signals.iter().any(|s| lower.contains(s));
    let real = real_signals.iter().any(|s| lower.contains(s));

    let (verdict, reason) = match (flaky, real) {
        (true, false) => (
            ClassifyVerdict::Flaky,
            "heuristic: transient/network/timeout signals in logs".into(),
        ),
        (false, true) => (
            ClassifyVerdict::Real,
            "heuristic: assertion/compile/test failure signals in logs".into(),
        ),
        (true, true) => (
            ClassifyVerdict::Real,
            "heuristic: mixed signals; defaulting to real bug".into(),
        ),
        (false, false) => (
            ClassifyVerdict::Unknown,
            "heuristic: could not classify; inspect logs manually".into(),
        ),
    };

    ClassifyResult {
        verdict,
        reason,
        test_name: extract_test_name(logs),
        used_llm: false,
        pages_read: 1,
        page_summary: None,
    }
}

fn extract_test_name(logs: &str) -> Option<String> {
    for line in logs.lines() {
        let t = line.trim();
        if t.contains("::") && (t.contains("FAILED") || t.contains("failed")) {
            return Some(t.chars().take(120).collect());
        }
        if t.starts_with("FAIL ") || t.starts_with("--- FAIL:") {
            return Some(t.chars().take(120).collect());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heuristic_flaky() {
        let r = heuristic_classify("ci", "Error: connect ETIMEDOUT after 30000ms");
        assert_eq!(r.verdict, ClassifyVerdict::Flaky);
    }

    #[test]
    fn heuristic_real() {
        let r = heuristic_classify("ci", "thread 'main' panicked at 'assertion failed'");
        assert_eq!(r.verdict, ClassifyVerdict::Real);
    }

    #[test]
    fn policy_workflow_by_name() {
        assert!(is_policy_workflow("Backport PR manager approval checker"));
        let r = quick_classify("Backport PR manager approval checker", "").unwrap();
        assert_eq!(r.verdict, ClassifyVerdict::Policy);
    }

    #[test]
    fn policy_workflow_by_log() {
        let r = quick_classify(
            "Some Check",
            "Error: PR is missing required label no-e2e",
        )
        .unwrap();
        assert_eq!(r.verdict, ClassifyVerdict::Policy);
    }
}
