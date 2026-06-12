use serde::{Deserialize, Serialize};

use crate::config::LlmConfig;
use crate::error::{CoworkerError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClassifyVerdict {
    Flaky,
    Real,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ClassifyResponse {
    verdict: ClassifyVerdict,
    reason: String,
    #[serde(default)]
    test_name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ClassifyResult {
    pub verdict: ClassifyVerdict,
    pub reason: String,
    pub test_name: Option<String>,
    pub used_llm: bool,
}

pub struct LlmClient {
    cfg: LlmConfig,
    http: reqwest::Client,
}

impl LlmClient {
    pub fn new(cfg: LlmConfig) -> Self {
        Self {
            cfg,
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .expect("reqwest client"),
        }
    }

    pub async fn available(&self) -> bool {
        crate::llm::ollama::probe(&self.cfg).await
    }

    pub async fn classify_ci_failure(
        &self,
        skill_body: &str,
        repo: &str,
        pr_number: u32,
        workflow: &str,
        logs: &str,
    ) -> Result<ClassifyResult> {
        if self.available().await {
            match self
                .classify_with_llm(skill_body, repo, pr_number, workflow, logs)
                .await
            {
                Ok(r) => return Ok(r),
                Err(e) => {
                    tracing::warn!("LLM classify failed, using heuristics: {e}");
                }
            }
        }
        Ok(heuristic_classify(logs))
    }

    async fn classify_with_llm(
        &self,
        skill_body: &str,
        repo: &str,
        pr_number: u32,
        workflow: &str,
        logs: &str,
    ) -> Result<ClassifyResult> {
        let system = format!(
            "{skill_body}\n\n\
You classify CI failures as flaky or real bugs. \
Respond with JSON only, no markdown: \
{{\"verdict\":\"flaky\"|\"real\"|\"unknown\",\"reason\":\"...\",\"test_name\":\"optional\"}}"
        );
        let user = format!(
            "repo: {repo}\npr: #{pr_number}\nworkflow: {workflow}\n\nFailed logs:\n{logs}"
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
        })
    }
}

pub fn heuristic_classify(logs: &str) -> ClassifyResult {
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
        let r = heuristic_classify("Error: connect ETIMEDOUT after 30000ms");
        assert_eq!(r.verdict, ClassifyVerdict::Flaky);
    }

    #[test]
    fn heuristic_real() {
        let r = heuristic_classify("thread 'main' panicked at 'assertion failed'");
        assert_eq!(r.verdict, ClassifyVerdict::Real);
    }
}
