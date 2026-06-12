use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;

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
    /// One-line summary for lists.
    reason: String,
    /// What failed and why, with log evidence (2–4 sentences).
    #[serde(default)]
    diagnosis: Option<String>,
    /// Concrete next step for the PR author.
    #[serde(default)]
    recommended_action: Option<String>,
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
    pub diagnosis: Option<String>,
    pub recommended_action: Option<String>,
    pub test_name: Option<String>,
    pub used_llm: bool,
    pub pages_read: u32,
    pub page_summary: Option<String>,
}

pub struct LlmClient {
    cfg: LlmConfig,
    http: reqwest::Client,
    online: bool,
    concurrency: Arc<Semaphore>,
}

impl LlmClient {
    pub fn new(cfg: LlmConfig, online: bool) -> Self {
        let permits = cfg.concurrency.max(1) as usize;
        Self {
            cfg,
            online,
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .expect("reqwest client"),
            concurrency: Arc::new(Semaphore::new(permits)),
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
        } else if let Some(result) = quick_classify(page_logs) {
            if result.verdict != ClassifyVerdict::Unknown {
                return Ok(ClassifyResult {
                    pages_read: page_num,
                    ..result
                });
            }
        }

        Ok(heuristic_classify(combined_logs))
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
        let _permit = self
            .concurrency
            .acquire()
            .await
            .map_err(|e| CoworkerError::Other(anyhow::anyhow!("llm concurrency: {e}")))?;

        let system = format!(
            "{skill_body}\n\n\
You triage CI failures for a daily digest. Classify each failure and explain it clearly.\n\
\n\
Verdicts:\n\
- flaky: transient infra/network/timeouts; rerun may pass\n\
- real: code/test/build bug in the PR\n\
- policy: labels, approvals, changelog, or template gates — not a test flake\n\
- unknown: logs insufficient on this page\n\
\n\
Always fill ALL fields with specific, actionable content from the logs:\n\
- reason: one-line summary (not vague phrases like \"not flaky\" alone)\n\
- diagnosis: 2–4 sentences — what check failed, quote or paraphrase log evidence, impact on merge\n\
- recommended_action: concrete next step for the PR author (fix code, add label, get approval, rerun, etc.)\n\
- test_name: failing test if identifiable\n\
\n\
You may receive one page of logs at a time; prior pages are summarized, not repeated. \
If inconclusive on this page, use verdict unknown and fill page_summary for the next page."
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

        let messages = serde_json::json!([
            {"role": "system", "content": system},
            {"role": "user", "content": user},
        ]);

        let content = if let Some(ollama_base) = ollama_api_base(&self.cfg.base_url) {
            self.chat_ollama_native(&ollama_base, &messages).await?
        } else {
            self.chat_openai_compatible(&messages).await?
        };

        let parsed = parse_classify_response(&content).map_err(|e| {
            CoworkerError::Other(anyhow::anyhow!("llm parse classify json: {e}; raw={content}"))
        })?;

        Ok(ClassifyResult {
            verdict: parsed.verdict,
            reason: parsed.reason,
            diagnosis: parsed.diagnosis,
            recommended_action: parsed.recommended_action,
            test_name: parsed.test_name,
            used_llm: true,
            pages_read: page_num,
            page_summary: parsed.page_summary,
        })
    }

    /// Ollama native API — schema in `format` is enforced more reliably than on `/v1`.
    async fn chat_ollama_native(&self, base: &str, messages: &serde_json::Value) -> Result<String> {
        let url = format!("{base}/api/chat");
        let mut body = serde_json::json!({
            "model": self.cfg.model,
            "messages": messages,
            "stream": false,
            "options": { "temperature": 0 },
        });
        apply_structured_format(&mut body, self.cfg.structured_output);

        let v = self.post_json(&url, &body).await?;
        v.pointer("/message/content")
            .and_then(|c| c.as_str())
            .map(str::to_string)
            .ok_or_else(|| CoworkerError::Other(anyhow::anyhow!("ollama missing message.content")))
    }

    /// OpenAI-compatible `/v1/chat/completions` (OpenAI, vLLM, or Ollama fallback).
    async fn chat_openai_compatible(&self, messages: &serde_json::Value) -> Result<String> {
        let url = format!(
            "{}/chat/completions",
            self.cfg.base_url.trim_end_matches('/')
        );
        let mut body = serde_json::json!({
            "model": self.cfg.model,
            "messages": messages,
            "stream": false,
            "temperature": 0,
        });
        apply_structured_format(&mut body, self.cfg.structured_output);

        let v = self.post_json(&url, &body).await?;
        v.pointer("/choices/0/message/content")
            .and_then(|c| c.as_str())
            .map(str::to_string)
            .ok_or_else(|| CoworkerError::Other(anyhow::anyhow!("llm missing content")))
    }

    async fn post_json(&self, url: &str, body: &serde_json::Value) -> Result<serde_json::Value> {
        let resp = self
            .http
            .post(url)
            .json(body)
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

        resp.json()
            .await
            .map_err(|e| CoworkerError::Other(anyhow::anyhow!("llm json: {e}")))
    }
}

fn ollama_api_base(base_url: &str) -> Option<String> {
    let trimmed = base_url.trim_end_matches('/');
    if trimmed.ends_with("/v1") {
        return Some(trimmed.strip_suffix("/v1")?.to_string());
    }
    if trimmed.contains("11434") {
        return Some(trimmed.to_string());
    }
    None
}

fn classify_response_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "verdict": {
                "type": "string",
                "enum": ["flaky", "real", "policy", "unknown"]
            },
            "reason": {
                "type": "string",
                "description": "One-line summary"
            },
            "diagnosis": {
                "type": "string",
                "description": "2-4 sentences: what failed, log evidence, merge impact"
            },
            "recommended_action": {
                "type": "string",
                "description": "Concrete next step for the PR author"
            },
            "test_name": { "type": "string" },
            "page_summary": { "type": "string" }
        },
        "required": ["verdict", "reason", "diagnosis", "recommended_action"],
        "additionalProperties": false
    })
}

/// Attach structured-output constraints for Ollama (`format`) and OpenAI (`response_format`).
fn apply_structured_format(body: &mut serde_json::Value, structured: bool) {
    let obj = body.as_object_mut().expect("request body object");
    if structured {
        let schema = classify_response_schema();
        obj.insert("format".into(), schema.clone());
        obj.insert(
            "response_format".into(),
            serde_json::json!({
                "type": "json_schema",
                "json_schema": {
                    "name": "classify_ci_failure",
                    "strict": true,
                    "schema": schema
                }
            }),
        );
    } else {
        obj.insert("format".into(), serde_json::Value::String("json".into()));
        obj.insert(
            "response_format".into(),
            serde_json::json!({ "type": "json_object" }),
        );
    }
}

/// Strip markdown fences and parse classify JSON from model output.
fn parse_classify_response(content: &str) -> std::result::Result<ClassifyResponse, serde_json::Error> {
    let trimmed = content.trim();
    if let Ok(v) = serde_json::from_str::<ClassifyResponse>(trimmed) {
        return Ok(v);
    }

    let unfenced = strip_markdown_fence(trimmed);
    if unfenced != trimmed {
        if let Ok(v) = serde_json::from_str::<ClassifyResponse>(&unfenced) {
            return Ok(v);
        }
    }

    if let Some(json) = extract_json_object(trimmed) {
        return serde_json::from_str(&json);
    }

    serde_json::from_str(trimmed)
}

fn strip_markdown_fence(s: &str) -> String {
    let s = s.trim();
    let Some(rest) = s.strip_prefix("```") else {
        return s.to_string();
    };
    let rest = rest.trim_start();
    let rest = rest
        .strip_prefix("json")
        .or_else(|| rest.strip_prefix("JSON"))
        .unwrap_or(rest)
        .trim_start();
    let body = rest.strip_suffix("```").unwrap_or(rest).trim();
    body.to_string()
}

fn extract_json_object(s: &str) -> Option<String> {
    let start = s.find('{')?;
    let end = s.rfind('}')?;
    if end > start {
        Some(s[start..=end].to_string())
    } else {
        None
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

/// Digest lines for a classified CI run (multi-line: header + diagnosis + action).
pub fn format_classify_digest_lines(
    run_id: i64,
    workflow: &str,
    classify: &ClassifyResult,
) -> Vec<String> {
    let verdict = verdict_label(classify.verdict);
    let source = if classify.used_llm { "llm" } else { "heuristic" };
    let mut lines = vec![format!(
        "- run {run_id} {workflow} → {verdict} ({} page(s), {source})",
        classify.pages_read
    )];

    let diagnosis = classify
        .diagnosis
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(classify.reason.trim());
    if !diagnosis.is_empty() {
        lines.push(format!("  Diagnosis: {diagnosis}"));
    }

    if let Some(action) = classify.recommended_action.as_deref() {
        let action = action.trim();
        if !action.is_empty() {
            lines.push(format!("  Action: {action}"));
        }
    }

    if let Some(test) = classify.test_name.as_deref() {
        let test = test.trim();
        if !test.is_empty() {
            lines.push(format!("  Test: {test}"));
        }
    }

    lines
}

fn verdict_label(v: ClassifyVerdict) -> &'static str {
    match v {
        ClassifyVerdict::Flaky => "flaky",
        ClassifyVerdict::Real => "real bug",
        ClassifyVerdict::Policy => "policy",
        ClassifyVerdict::Unknown => "unknown",
    }
}

pub fn llm_reason_text(classify: &ClassifyResult) -> String {
    let diagnosis = classify
        .diagnosis
        .as_deref()
        .unwrap_or(&classify.reason);
    let action = classify
        .recommended_action
        .as_deref()
        .unwrap_or("(no action given)");
    format!(
        "{diagnosis} | Action: {action} ({})",
        if classify.used_llm { "llm" } else { "heuristic" }
    )
}

/// Fast path for offline heuristics: log shape only (never workflow name).
pub fn quick_classify(logs: &str) -> Option<ClassifyResult> {
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
            diagnosis: Some(heuristic_diagnosis(logs, ClassifyVerdict::Policy)),
            recommended_action: Some(heuristic_action(ClassifyVerdict::Policy)),
            test_name: None,
            used_llm: false,
            pages_read: 1,
            page_summary: None,
        });
    }
    None
}

pub fn heuristic_classify(logs: &str) -> ClassifyResult {
    if let Some(r) = quick_classify(logs) {
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
            "heuristic: transient/network/timeout signals in logs".to_string(),
        ),
        (false, true) => (
            ClassifyVerdict::Real,
            "heuristic: assertion/compile/test failure signals in logs".to_string(),
        ),
        (true, true) => (
            ClassifyVerdict::Real,
            "heuristic: mixed signals; defaulting to real bug".to_string(),
        ),
        (false, false) => (
            ClassifyVerdict::Unknown,
            "heuristic: could not classify; inspect logs manually".to_string(),
        ),
    };

    ClassifyResult {
        verdict,
        reason,
        diagnosis: Some(heuristic_diagnosis(logs, verdict)),
        recommended_action: Some(heuristic_action(verdict)),
        test_name: extract_test_name(logs),
        used_llm: false,
        pages_read: 1,
        page_summary: None,
    }
}

fn heuristic_diagnosis(logs: &str, verdict: ClassifyVerdict) -> String {
    let signals: Vec<_> = logs
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .filter(|l| {
            let t = l.to_ascii_lowercase();
            t.contains("error")
                || t.contains("fail")
                || t.contains("panic")
                || t.contains("approval")
                || t.contains("label")
                || t.contains("changelog")
        })
        .take(4)
        .collect();

    let excerpt = if signals.is_empty() {
        logs.lines().take(3).collect::<Vec<_>>().join(" | ")
    } else {
        signals.join(" | ")
    };

    format!(
        "Heuristic {} from log excerpt: {}",
        verdict_label(verdict),
        excerpt.chars().take(400).collect::<String>()
    )
}

fn heuristic_action(verdict: ClassifyVerdict) -> String {
    match verdict {
        ClassifyVerdict::Flaky => {
            "Likely transient — rerun the workflow; if green, no code change needed.".into()
        }
        ClassifyVerdict::Real => {
            "Fix the failing test/build locally, push a commit, and wait for CI.".into()
        }
        ClassifyVerdict::Policy => {
            "Resolve the policy gate (label, approval, changelog, or PR template) on the PR.".into()
        }
        ClassifyVerdict::Unknown => {
            "Open the failing run on GitHub and inspect the full log.".into()
        }
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

    #[test]
    fn quick_classify_ignores_workflow_name() {
        assert!(quick_classify("").is_none());
    }

    #[test]
    fn policy_by_log() {
        let r = quick_classify("Error: PR is missing required label no-e2e").unwrap();
        assert_eq!(r.verdict, ClassifyVerdict::Policy);
    }

    #[test]
    fn ollama_api_base_from_v1_url() {
        assert_eq!(
            ollama_api_base("http://localhost:11434/v1").as_deref(),
            Some("http://localhost:11434")
        );
    }

    #[test]
    fn structured_format_uses_schema() {
        let mut body = serde_json::json!({"model": "m"});
        apply_structured_format(&mut body, true);
        assert!(body.get("format").unwrap().get("properties").is_some());
        assert_eq!(
            body.pointer("/response_format/type").and_then(|v| v.as_str()),
            Some("json_schema")
        );
    }

    #[test]
    fn parse_classify_from_markdown_fence() {
        let raw = "```json\n{\"verdict\":\"policy\",\"reason\":\"needs approval\"}\n```";
        let r = parse_classify_response(raw).unwrap();
        assert_eq!(r.verdict, ClassifyVerdict::Policy);
    }

    #[test]
    fn parse_classify_from_plain_json() {
        let raw = r#"{"verdict":"real","reason":"compile error","diagnosis":"Build failed in pkg/foo","recommended_action":"Fix imports"}"#;
        let r = parse_classify_response(raw).unwrap();
        assert_eq!(r.verdict, ClassifyVerdict::Real);
        assert_eq!(r.diagnosis.as_deref(), Some("Build failed in pkg/foo"));
    }

    #[test]
    fn digest_lines_include_diagnosis() {
        let c = ClassifyResult {
            verdict: ClassifyVerdict::Policy,
            reason: "approval missing".into(),
            diagnosis: Some("Backport requires manager approval per policy link.".into()),
            recommended_action: Some("Ping manager for approval.".into()),
            test_name: None,
            used_llm: true,
            pages_read: 1,
            page_summary: None,
        };
        let lines = format_classify_digest_lines(123, "Backport checker", &c);
        assert!(lines.iter().any(|l| l.contains("Diagnosis:")));
        assert!(lines.iter().any(|l| l.contains("Action:")));
    }
}
