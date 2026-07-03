use std::sync::Arc;
use std::sync::LazyLock;

use regex::Regex;
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

impl ClassifyResponse {
    fn sanitized(mut self) -> Self {
        self.reason = sanitize_llm_field(&self.reason);
        self.diagnosis = self
            .diagnosis
            .as_deref()
            .map(sanitize_llm_field)
            .filter(|s| !s.is_empty());
        self.recommended_action = self
            .recommended_action
            .as_deref()
            .map(sanitize_llm_field)
            .filter(|s| !s.is_empty());
        self.test_name = self
            .test_name
            .as_deref()
            .map(sanitize_llm_field)
            .filter(|s| !s.is_empty());
        self.page_summary = self
            .page_summary
            .as_deref()
            .map(sanitize_llm_field)
            .filter(|s| !s.is_empty());
        self
    }

    fn into_classify_result(self, pages_read: u32, truncated: bool) -> ClassifyResult {
        let reason = if truncated {
            mark_partial_llm_output(self.reason)
        } else {
            self.reason
        };
        ClassifyResult {
            verdict: self.verdict,
            reason,
            diagnosis: self.diagnosis,
            recommended_action: self.recommended_action,
            test_name: self.test_name,
            used_llm: true,
            pages_read,
            page_summary: self.page_summary,
        }
    }
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
    /// Longer timeout for Ollama chat streaming (thinking models can run for minutes).
    http_stream: reqwest::Client,
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
            http_stream: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(600))
                .build()
                .expect("reqwest stream client"),
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

        let system = format!("{skill_body}{}", thinking_prompt_suffix(&self.cfg));
        let concise_suffix = "\n\nIMPORTANT: Keep JSON compact. Do not exceed field length limits or the response will be truncated.";
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

        let mut retried_without_think = false;
        let mut retried_concise = false;

        loop {
            let msgs = if retried_concise {
                serde_json::json!([
                    {"role": "system", "content": format!("{system}{concise_suffix}")},
                    {"role": "user", "content": user},
                ])
            } else {
                messages.clone()
            };

            let content = self.chat_openai_compatible(&msgs).await?;

            if content.trim().is_empty() {
                if self.cfg.think && !retried_without_think {
                    tracing::warn!(
                        "LLM classify page {page_num}: empty LLM output, retrying with think=false"
                    );
                    retried_without_think = true;
                    continue;
                }
                return Err(CoworkerError::Other(anyhow::anyhow!(
                    "llm empty classify output; raw={content}"
                )));
            }

            match parse_classify_response(&content) {
                Ok(parsed) => {
                    if retried_without_think && self.cfg.think {
                        tracing::info!(
                            "LLM classify page {page_num}: succeeded after think=false retry"
                        );
                    }
                    return Ok(parsed.into_classify_result(page_num, false));
                }
                Err(e) => {
                    if let Some(parsed) = salvage_truncated_classify(&content) {
                        tracing::info!(
                            "LLM classify page {page_num}: recovered partial JSON after parse error: {e}"
                        );
                        return Ok(parsed.into_classify_result(page_num, true));
                    }
                    if self.cfg.think && !retried_without_think {
                        tracing::warn!(
                            "LLM classify page {page_num} parse failed, retrying with think=false: {e}"
                        );
                        retried_without_think = true;
                        continue;
                    }
                    if !retried_concise {
                        tracing::warn!(
                            "LLM classify page {page_num} parse failed, retrying with concise prompt: {e}"
                        );
                        retried_concise = true;
                        continue;
                    }
                    return Err(CoworkerError::Other(anyhow::anyhow!(
                        "llm parse classify json: {e}; raw={content}"
                    )));
                }
            }
        }
    }

    fn llm_output_limit(&self) -> u32 {
        self.cfg.max_output_tokens.max(256)
    }

    /// OpenAI-compatible `/v1/chat/completions` (OpenAI, vLLM, oMLX, Ollama `/v1`, etc.).
    async fn chat_openai_compatible(&self, messages: &serde_json::Value) -> Result<String> {
        self.chat_openai_structured(
            messages,
            &classify_response_schema(),
            self.llm_output_limit(),
        )
        .await
    }

    async fn chat_openai_structured(
        &self,
        messages: &serde_json::Value,
        _schema: &serde_json::Value,
        max_tokens: u32,
    ) -> Result<String> {
        let url = openai_chat_completions_url(&self.cfg.base_url);
        let mut body = serde_json::json!({
            "model": self.cfg.model,
            "messages": messages,
            "stream": false,
            "temperature": 0,
            "max_tokens": max_tokens,
        });
        apply_openai_json_mode(&mut body);

        let v = self.post_json(&url, &body).await?;
        extract_openai_chat_content(&v)
    }

    async fn post_json(&self, url: &str, body: &serde_json::Value) -> Result<serde_json::Value> {
        let resp = crate::llm::ollama::apply_llm_auth(self.http.post(url), &self.cfg)
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

    /// LLM gate for `bash_run` (think=false, JSON verdict).
    pub async fn review_bash_command_json(
        &self,
        prompt_template: &str,
        command: &str,
        schema: &serde_json::Value,
        max_tokens: u32,
    ) -> Result<String> {
        self.review_code_snippet_json(
            prompt_template,
            command,
            schema,
            max_tokens,
            "LLM offline — cannot review bash command",
        )
        .await
    }

    /// LLM gate for `python_run` (think=false, JSON verdict).
    pub async fn review_python_code_json(
        &self,
        prompt_template: &str,
        code: &str,
        schema: &serde_json::Value,
        max_tokens: u32,
    ) -> Result<String> {
        self.review_code_snippet_json(
            prompt_template,
            code,
            schema,
            max_tokens,
            "LLM offline — cannot review python code",
        )
        .await
    }

    /// LLM gate for `edit_file` / `write_file` (think=false, JSON verdict).
    pub async fn review_file_edit_json(
        &self,
        prompt_template: &str,
        payload: &str,
        schema: &serde_json::Value,
        max_tokens: u32,
    ) -> Result<String> {
        self.review_code_snippet_json(
            prompt_template,
            payload,
            schema,
            max_tokens,
            "LLM offline — cannot review file edit",
        )
        .await
    }

    /// LLM gate for chat turn completion (think=false, JSON verdict).
    pub async fn judge_chat_turn_complete_json(
        &self,
        prompt_template: &str,
        payload: &str,
        schema: &serde_json::Value,
        max_tokens: u32,
    ) -> Result<String> {
        self.review_code_snippet_json(
            prompt_template,
            payload,
            schema,
            max_tokens,
            "LLM offline — cannot judge chat turn completion",
        )
        .await
    }

    async fn review_code_snippet_json(
        &self,
        prompt_template: &str,
        snippet: &str,
        schema: &serde_json::Value,
        max_tokens: u32,
        offline_message: &str,
    ) -> Result<String> {
        if !self.online {
            return Err(CoworkerError::Other(anyhow::anyhow!("{offline_message}")));
        }
        let _permit = self
            .concurrency
            .acquire()
            .await
            .map_err(|e| CoworkerError::Other(anyhow::anyhow!("llm concurrency: {e}")))?;

        let user_content = format!("{}\n{}", prompt_template.trim_end(), snippet);
        let messages = serde_json::json!([{"role": "user", "content": user_content}]);
        let think_override = Some(false);
        let tokens = max_tokens.clamp(256, 2048);

        let structured = self.chat_openai_structured(&messages, schema, tokens).await;
        match structured {
            Ok(content) => Ok(content),
            Err(e) if llm_structured_output_unavailable(&e) => {
                tracing::debug!(
                    "structured JSON output unavailable ({e}) — retrying review call as plain text"
                );
                self.chat_plain_messages(&messages, tokens, think_override)
                    .await
            }
            Err(e) => Err(e),
        }
    }

    /// Multi-turn chat using native `tools` / `tool_calls`.
    pub async fn complete_chat_with_tools_with_progress<F>(
        &self,
        messages: &serde_json::Value,
        tools: &[serde_json::Value],
        cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
        reasoning_only_warn_secs: u64,
        mut on_buffer: F,
    ) -> Result<super::chat::ChatToolsTurn>
    where
        F: FnMut(&str, &str, &[super::chat::LlmToolCall]) + Send,
    {
        if !self.online {
            return Err(CoworkerError::Other(anyhow::anyhow!(
                "LLM offline — check llm.base_url and that the server is running \
                 (set llm.api_key for LLM Provider)"
            )));
        }
        if tools.is_empty() {
            return Err(CoworkerError::Other(anyhow::anyhow!(
                "chat tools list is empty"
            )));
        }

        let _permit = self
            .concurrency
            .acquire()
            .await
            .map_err(|e| CoworkerError::Other(anyhow::anyhow!("llm concurrency: {e}")))?;

        let base_limit = self.chat_output_limit();
        let attempts: [(Option<bool>, u32); 3] = [
            (if self.cfg.think { None } else { Some(false) }, base_limit),
            (Some(false), base_limit.saturating_mul(2).min(8192)),
            (Some(false), 8192),
        ];
        let attempt_count = attempts.len();

        let mut last = super::chat::ChatToolsTurn::default();
        for (i, (think_override, num_predict)) in attempts.into_iter().enumerate() {
            let mut msgs = messages.clone();
            let think_was_on = think_override.unwrap_or(self.cfg.think);
            if i > 0 {
                let nudge = if last.content.trim().is_empty() && last.tool_calls.is_empty() {
                    "Your last turn returned no assistant text and no tool_calls. \
Call one or more tools or reply to the user in plain text."
                        .to_string()
                } else if last.tool_calls.is_empty() {
                    "Your last turn was a status update without tool_calls. \
Call tools to continue, or reply with a complete synthesis when the task is done."
                        .to_string()
                } else {
                    "Your last turn was incomplete. Call tool(s) or reply with a complete answer."
                        .to_string()
                };
                msgs.as_array_mut().unwrap().push(serde_json::json!({
                    "role": "user",
                    "content": nudge,
                }));
            }
            let turn = match self
                .chat_openai_with_tools_stream(
                    &msgs,
                    tools,
                    num_predict,
                    cancel.clone(),
                    reasoning_only_warn_secs,
                    &mut on_buffer,
                )
                .await
            {
                Ok(turn) => turn,
                Err(e) if i + 1 < attempt_count => {
                    tracing::warn!("chat tools stream failed ({e}); retrying");
                    last = super::chat::ChatToolsTurn::default();
                    continue;
                }
                Err(e) => return Err(e),
            };
            last = turn.clone();
            let needs_retry = super::chat::agent_tools_turn_needs_retry(&last, think_was_on);
            if needs_retry && i + 1 < attempt_count {
                tracing::warn!("chat tools turn incomplete; retrying without think");
                continue;
            }
            if last.content.trim().is_empty() && last.tool_calls.is_empty() {
                return Err(CoworkerError::Other(anyhow::anyhow!(
                    "llm returned empty content and no tool_calls after {attempt_count} attempts"
                )));
            }
            return Ok(turn);
        }
        Ok(last)
    }

    fn chat_output_limit(&self) -> u32 {
        self.cfg.max_output_tokens.clamp(2048, 8192)
    }

    #[allow(clippy::too_many_arguments)]
    async fn chat_openai_with_tools_stream<F>(
        &self,
        messages: &serde_json::Value,
        tools: &[serde_json::Value],
        num_predict: u32,
        cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
        reasoning_only_warn_secs: u64,
        on_buffer: &mut F,
    ) -> Result<super::chat::ChatToolsTurn>
    where
        F: FnMut(&str, &str, &[super::chat::LlmToolCall]) + Send,
    {
        use futures_util::StreamExt;

        let url = openai_chat_completions_url(&self.cfg.base_url);
        let body = serde_json::json!({
            "model": self.cfg.model,
            "messages": messages,
            "stream": true,
            "temperature": 0,
            "max_tokens": num_predict,
            "tools": tools,
            "tool_choice": "auto",
        });

        let resp = crate::llm::ollama::apply_llm_auth(self.http_stream.post(&url), &self.cfg)
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

        let mut stream = resp.bytes_stream();
        let mut content = String::new();
        let mut reasoning = String::new();
        let mut tool_call_acc: Vec<serde_json::Value> = Vec::new();
        let mut line_buf = String::new();
        let idle_timeout = std::time::Duration::from_secs(30);
        let stream_wall_limit = std::time::Duration::from_secs(CHAT_STREAM_WALL_SECS);
        let stream_started = std::time::Instant::now();
        let mut reasoning_only_since: Option<std::time::Instant> = None;
        let mut stop_stream = false;

        while !stop_stream {
            if super::chat::chat_cancel_requested(&cancel) {
                return Err(super::chat::chat_cancelled_error());
            }
            if stream_wall_exceeded(
                stream_started.elapsed(),
                reasoning_only_since.map(|t| t.elapsed()),
                content.len(),
                reasoning.len(),
                tool_call_acc.len(),
                stream_wall_limit,
                reasoning_only_warn_secs,
            ) {
                let reason = if content.trim().is_empty() && !reasoning.trim().is_empty() {
                    "reasoning-only cap"
                } else {
                    "stream wall"
                };
                tracing::warn!(
                    "openai chat tools stream {reason} (reasoning {} chars, content {} chars)",
                    reasoning.len(),
                    content.len()
                );
                break;
            }
            let chunk_result = match tokio::time::timeout(idle_timeout, stream.next()).await {
                Ok(result) => result,
                Err(_) => {
                    tracing::warn!(
                        "openai chat tools stream idle {}s (reasoning {} chars, content {} chars)",
                        idle_timeout.as_secs(),
                        reasoning.len(),
                        content.len()
                    );
                    break;
                }
            };
            if super::chat::chat_cancel_requested(&cancel) {
                return Err(super::chat::chat_cancelled_error());
            }
            let chunk = match chunk_result {
                Some(Ok(chunk)) => chunk,
                Some(Err(e)) => {
                    return Err(CoworkerError::Other(anyhow::anyhow!("llm stream: {e}")));
                }
                None => break,
            };
            line_buf.push_str(&String::from_utf8_lossy(&chunk));
            while let Some(pos) = line_buf.find('\n') {
                let line = line_buf[..pos].trim().to_string();
                line_buf = line_buf[pos + 1..].to_string();
                if line.is_empty() || line.starts_with(':') {
                    continue;
                }
                let data = line.strip_prefix("data:").unwrap_or(&line).trim();
                if data.is_empty() || data == "[DONE]" {
                    continue;
                }
                let v: serde_json::Value = serde_json::from_str(data).map_err(|e| {
                    CoworkerError::Other(anyhow::anyhow!("openai stream json: {e}; line={line}"))
                })?;
                let Some(delta) = v.pointer("/choices/0/delta") else {
                    continue;
                };
                let mut changed = false;
                if let Some(part) = delta
                    .get("reasoning_content")
                    .or_else(|| delta.get("reasoning"))
                    .and_then(|t| t.as_str())
                {
                    if !part.is_empty() {
                        append_stream_text(&mut reasoning, part);
                        changed = true;
                    }
                }
                if let Some(part) = delta.get("content").and_then(|c| c.as_str()) {
                    if !part.is_empty() {
                        append_stream_text(&mut content, part);
                        changed = true;
                    }
                }
                if let Some(calls) = delta.get("tool_calls").and_then(|a| a.as_array()) {
                    merge_openai_tool_call_deltas(&mut tool_call_acc, calls);
                    changed = true;
                }
                if changed {
                    let tool_calls = parse_accumulated_openai_tool_calls(&tool_call_acc);
                    on_buffer(&content, &reasoning, &tool_calls);
                }
                if content.trim().is_empty() && !reasoning.trim().is_empty() {
                    if reasoning_only_since.is_none() {
                        reasoning_only_since = Some(std::time::Instant::now());
                    }
                    if stream_text_appears_stuck(&reasoning) {
                        tracing::warn!(
                            "openai chat tools stream aborted: reasoning loop (~{} chars)",
                            reasoning.len()
                        );
                        stop_stream = true;
                        break;
                    }
                } else if !content.trim().is_empty() {
                    reasoning_only_since = None;
                }
            }
        }

        let tool_calls = parse_accumulated_openai_tool_calls(&tool_call_acc);
        if content.trim().is_empty() && tool_calls.is_empty() && reasoning.trim().is_empty() {
            return Err(CoworkerError::Other(anyhow::anyhow!(
                "llm empty message content"
            )));
        }
        Ok(super::chat::ChatToolsTurn {
            content,
            reasoning,
            tool_calls,
        })
    }

    /// Compress a long thinking trace into bullet lines for chat context (always think=false).
    pub async fn summarize_reasoning_trace(&self, reasoning: &str) -> Result<String> {
        const SYSTEM: &str =
            "You compress internal agent reasoning into 2-5 short bullet lines for \
later context. Keep PR numbers, tool names, run IDs, and conclusions. \
Output plain-text bullets only — do NOT call tools or emit tool_calls JSON. \
No preamble or markdown fences.";
        let trimmed = reasoning.trim();
        if trimmed.is_empty() {
            return Ok(String::new());
        }
        let user = format!(
            "Summarize this past agent reasoning trace (read-only):\n\n---\n{}\n---",
            trimmed.chars().take(12_000).collect::<String>()
        );
        if !self.online {
            return Err(CoworkerError::Other(anyhow::anyhow!(
                "LLM offline — cannot compress reasoning"
            )));
        }
        let _permit = self
            .concurrency
            .acquire()
            .await
            .map_err(|e| CoworkerError::Other(anyhow::anyhow!("llm concurrency: {e}")))?;

        let messages = serde_json::json!([
            { "role": "system", "content": SYSTEM },
            { "role": "user", "content": user },
        ]);
        let limit = self.cfg.reasoning_summary_tokens.clamp(256, 768).max(512);
        match self
            .chat_plain_messages(&messages, limit, Some(false))
            .await
        {
            Ok(summary) if !summary.trim().is_empty() => Ok(summary.trim().to_string()),
            Ok(_) => Err(CoworkerError::Other(anyhow::anyhow!(
                "reasoning summarizer returned empty content"
            ))),
            Err(e) => Err(e),
        }
    }

    /// Rolling summary of older chat turns (think=false).
    pub async fn summarize_session_history(&self, history_text: &str) -> Result<String> {
        self.summarize_session_history_with_prompt(
            history_text,
            "Summarize this chat session excerpt in 3-6 bullet lines for \
later context. Keep PR numbers, decisions, tool outcomes, and open questions. No preamble.",
        )
        .await
    }

    /// Coding chat rolling summary — preserve paths, errors, recent edits.
    pub async fn summarize_coding_session_history(&self, history_text: &str) -> Result<String> {
        self.summarize_session_history_with_prompt(
            history_text,
            "Summarize this coding chat excerpt in 3-6 bullet lines for later context. \
KEEP verbatim: file paths with line numbers (path:line), compile/test error text, \
recently edited file list, and the last tool conclusion. \
COMPRESS: long read_file/grep output, duplicate grep hits, old bash stdout (note exit code only). \
No preamble.",
        )
        .await
    }

    /// Ops / MCP triage rolling summary — preserve CI_KIND, verdicts, PR refs.
    pub async fn summarize_ops_session_history(&self, history_text: &str) -> Result<String> {
        self.summarize_session_history_with_prompt(
            history_text,
            "Summarize this GitHub ops chat excerpt in 3-6 bullet lines for later context. \
KEEP verbatim: CI_KIND lines, verdict (flaky/real/policy/unknown), owner/repo#N PR refs, \
digest counts (attention/flaky/policy), failing workflow names, and triage conclusions. \
COMPRESS: raw log excerpts and duplicate tool dumps. No preamble.",
        )
        .await
    }

    async fn summarize_session_history_with_prompt(
        &self,
        history_text: &str,
        system: &str,
    ) -> Result<String> {
        let user = history_text.chars().take(10_000).collect::<String>();
        if user.trim().is_empty() {
            return Ok(String::new());
        }
        if !self.online {
            return Ok(crate::agent::context::truncate_reasoning_local(&user, 320));
        }
        let _permit = self
            .concurrency
            .acquire()
            .await
            .map_err(|e| CoworkerError::Other(anyhow::anyhow!("llm concurrency: {e}")))?;
        let messages = serde_json::json!([
            { "role": "system", "content": system },
            { "role": "user", "content": user },
        ]);
        let limit = self.cfg.history_summary_tokens.clamp(128, 400);
        match self
            .chat_plain_messages(&messages, limit, Some(false))
            .await
        {
            Ok(summary) if !summary.trim().is_empty() => Ok(summary.trim().to_string()),
            Ok(_) => {
                tracing::warn!(
                    "session history summarizer returned empty content; using local truncation"
                );
                Ok(crate::agent::context::truncate_reasoning_local(&user, 320))
            }
            Err(e) => {
                tracing::warn!(
                    "session history summarizer LLM failed ({e}); using local truncation"
                );
                Ok(crate::agent::context::truncate_reasoning_local(&user, 320))
            }
        }
    }

    /// Rolling summary of older chat turns (think=false) — ops path.
    #[allow(dead_code)]
    pub async fn summarize_session_history_legacy(&self, history_text: &str) -> Result<String> {
        self.summarize_session_history(history_text).await
    }

    /// Plain-text chat completion — no JSON schema.
    async fn chat_plain_messages(
        &self,
        messages: &serde_json::Value,
        num_predict: u32,
        _think: Option<bool>,
    ) -> Result<String> {
        let url = openai_chat_completions_url(&self.cfg.base_url);
        let body = serde_json::json!({
            "model": self.cfg.model,
            "messages": messages,
            "stream": false,
            "temperature": 0,
            "max_tokens": num_predict,
            "tool_choice": "none",
        });
        let v = self.post_json(&url, &body).await?;
        extract_openai_plain_content(&v)
    }
}

fn openai_chat_completions_url(base_url: &str) -> String {
    format!(
        "{}/chat/completions",
        crate::llm::ollama::openai_v1_base(base_url)
    )
}

fn thinking_prompt_suffix(cfg: &LlmConfig) -> String {
    if !cfg.think {
        return String::new();
    }
    let word_budget = (cfg.max_thinking_tokens / 4).max(32);
    format!(
        "\n\n\
Before answering, reason step-by-step internally, but keep reasoning under ~{} tokens (~{} words). \
Focus on log evidence and verdict choice; do not restate the reasoning in the JSON fields.\n",
        cfg.max_thinking_tokens, word_budget
    )
}

#[allow(dead_code)] // unit tests; non-stream OpenAI fallback helper
fn parse_openai_tools_turn(v: &serde_json::Value) -> Result<super::chat::ChatToolsTurn> {
    let choice = v
        .pointer("/choices/0")
        .ok_or_else(|| CoworkerError::Other(anyhow::anyhow!("llm missing choices[0]")))?;
    let msg = choice
        .get("message")
        .ok_or_else(|| CoworkerError::Other(anyhow::anyhow!("llm missing choices[0].message")))?;
    let tool_calls = msg
        .get("tool_calls")
        .and_then(|a| a.as_array())
        .map(|arr| parse_native_tool_calls_from_array(arr))
        .unwrap_or_default();
    let reasoning = extract_openai_message_reasoning(msg);
    let content = extract_openai_visible_content(msg);
    if content.trim().is_empty() && tool_calls.is_empty() && reasoning.trim().is_empty() {
        let finish = choice
            .get("finish_reason")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        tracing::warn!(
            "openai chat tools empty response (finish_reason={finish}): {}",
            serde_json::to_string(msg).unwrap_or_default()
        );
        return Err(CoworkerError::Other(anyhow::anyhow!(
            "llm empty message content"
        )));
    }
    Ok(super::chat::ChatToolsTurn {
        content,
        reasoning,
        tool_calls,
    })
}

fn extract_openai_message_reasoning(msg: &serde_json::Value) -> String {
    for key in ["reasoning_content", "reasoning", "thinking"] {
        let text = openai_message_text_field(msg, key);
        if !text.trim().is_empty() {
            return text.trim().to_string();
        }
    }
    String::new()
}

/// User-visible assistant text only (not internal reasoning).
fn extract_openai_visible_content(msg: &serde_json::Value) -> String {
    let text = openai_message_text_field(msg, "content");
    if text.trim().is_empty() {
        String::new()
    } else {
        text
    }
}

fn openai_message_text_field(msg: &serde_json::Value, key: &str) -> String {
    let Some(value) = msg.get(key) else {
        return String::new();
    };
    if let Some(text) = value.as_str() {
        return text.to_string();
    }
    if let Some(parts) = value.as_array() {
        let mut out = String::new();
        for part in parts {
            if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                out.push_str(text);
            } else if let Some(text) = part.as_str() {
                out.push_str(text);
            }
        }
        return out;
    }
    String::new()
}

fn is_degenerate_plain_llm_text(text: &str) -> bool {
    let t = text.trim();
    if t.is_empty() {
        return true;
    }
    let thought_hits = t.matches("<thought").count();
    thought_hits >= 3 || (thought_hits >= 1 && t.len() < 120)
}

fn summarize_openai_tool_calls_for_plain(msg: &serde_json::Value) -> Option<String> {
    let calls = msg.get("tool_calls")?.as_array()?;
    if calls.is_empty() {
        return None;
    }
    let mut lines = Vec::new();
    for call in calls.iter().take(5) {
        let name = call
            .pointer("/function/name")
            .and_then(|v| v.as_str())
            .unwrap_or("tool");
        let args = call
            .pointer("/function/arguments")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let short_name = name.rsplit([':', '.']).next().unwrap_or(name);
        lines.push(format!("- Planned tool: {short_name}({args})"));
    }
    Some(lines.join("\n"))
}

fn extract_openai_message_content(msg: &serde_json::Value) -> String {
    let visible = extract_openai_visible_content(msg);
    if !visible.trim().is_empty() {
        return visible;
    }

    let reasoning = extract_openai_message_reasoning(msg);
    for key in ["reasoning_content", "reasoning", "thinking"] {
        if let Some(text) = non_empty_chat_fallback(&reasoning, key) {
            return text;
        }
    }

    String::new()
}

fn extract_openai_chat_content(v: &serde_json::Value) -> Result<String> {
    let msg = v
        .pointer("/choices/0/message")
        .ok_or_else(|| CoworkerError::Other(anyhow::anyhow!("llm missing choices[0].message")))?;

    let content = extract_openai_message_content(msg);
    if content.trim().is_empty() {
        return Err(CoworkerError::Other(anyhow::anyhow!(
            "llm empty message content"
        )));
    }
    Ok(content)
}

/// Plain-text OpenAI completion — accepts reasoning fields when thinking models
/// emit summaries only in `reasoning_content` (common on oMLX / Qwen-style APIs).
fn extract_openai_plain_content(v: &serde_json::Value) -> Result<String> {
    let msg = v
        .pointer("/choices/0/message")
        .ok_or_else(|| CoworkerError::Other(anyhow::anyhow!("llm missing choices[0].message")))?;

    let visible = extract_openai_visible_content(msg);
    if !is_degenerate_plain_llm_text(&visible) {
        return Ok(visible.trim().to_string());
    }

    let reasoning = extract_openai_message_reasoning(msg);
    if !is_degenerate_plain_llm_text(&reasoning) {
        tracing::debug!("openai plain reply recovered from reasoning field");
        return Ok(reasoning.trim().to_string());
    }

    if let Some(summary) = summarize_openai_tool_calls_for_plain(msg) {
        tracing::debug!("openai plain reply recovered from tool_calls plan");
        return Ok(summary);
    }

    let finish = v
        .pointer("/choices/0/finish_reason")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    Err(CoworkerError::Other(anyhow::anyhow!(
        "llm empty message content (finish_reason={finish})"
    )))
}

/// When thinking models exhaust `num_predict`, JSON may only appear in thinking/reasoning.
fn non_empty_chat_fallback(text: &str, field: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(json) = extract_json_object(trimmed) {
        tracing::warn!("llm message.content empty; recovered JSON from {field}");
        return Some(json);
    }
    if trimmed.contains('{') {
        tracing::warn!("llm message.content empty; using {field} as fallback");
        return Some(trimmed.to_string());
    }
    None
}

const CHAT_STREAM_WALL_SECS: u64 = 90;

/// Stop streaming when the full wall is hit, or sooner when only reasoning grows.
pub fn stream_wall_exceeded(
    stream_elapsed: std::time::Duration,
    reasoning_only_elapsed: Option<std::time::Duration>,
    content_len: usize,
    reasoning_len: usize,
    tool_calls_len: usize,
    full_wall: std::time::Duration,
    reasoning_only_warn_secs: u64,
) -> bool {
    if stream_elapsed >= full_wall {
        return true;
    }
    if reasoning_only_warn_secs == 0 || content_len > 0 || reasoning_len == 0 || tool_calls_len > 0
    {
        return false;
    }
    reasoning_only_elapsed
        .is_some_and(|e| e >= std::time::Duration::from_secs(reasoning_only_warn_secs))
}

/// Merge stream chunks that may be delta or cumulative (full prefix) updates.
fn append_stream_text(acc: &mut String, part: &str) {
    if part.is_empty() {
        return;
    }
    if acc.is_empty() {
        acc.push_str(part);
        return;
    }
    if part.len() >= acc.len() && part.starts_with(acc.as_str()) {
        *acc = part.to_string();
        return;
    }
    acc.push_str(part);
}

/// Detect models that loop the same reasoning paragraph without emitting JSON content.
fn stream_text_appears_stuck(text: &str) -> bool {
    let chars: Vec<char> = text.trim().chars().collect();
    if chars.len() < 180 {
        return false;
    }
    for win in [48_usize, 64, 72] {
        if chars.len() < win * 2 {
            continue;
        }
        let suffix = &chars[chars.len() - win..];
        let prior = &chars[chars.len() - win * 2..chars.len() - win];
        if suffix == prior {
            return true;
        }
    }
    false
}

fn merge_openai_tool_call_deltas(acc: &mut Vec<serde_json::Value>, deltas: &[serde_json::Value]) {
    for delta in deltas {
        let idx = delta.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        while acc.len() <= idx {
            acc.push(serde_json::json!({
                "id": "",
                "type": "function",
                "function": { "name": "", "arguments": "" }
            }));
        }
        let slot = &mut acc[idx];
        if let Some(id) = delta.get("id").and_then(|v| v.as_str()) {
            if !id.is_empty() {
                slot["id"] = serde_json::Value::String(id.to_string());
            }
        }
        if let Some(func) = delta.get("function") {
            if let Some(name) = func.get("name").and_then(|v| v.as_str()) {
                if !name.is_empty() {
                    slot["function"]["name"] = serde_json::Value::String(name.to_string());
                }
            }
            if let Some(args) = func.get("arguments").and_then(|v| v.as_str()) {
                let existing = slot["function"]["arguments"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();
                slot["function"]["arguments"] =
                    serde_json::Value::String(format!("{existing}{args}"));
            }
        }
    }
}

fn parse_accumulated_openai_tool_calls(
    chunks: &[serde_json::Value],
) -> Vec<super::chat::LlmToolCall> {
    if chunks.is_empty() {
        return Vec::new();
    }
    parse_native_tool_calls_from_array(chunks)
}

fn parse_native_tool_calls_from_array(arr: &[serde_json::Value]) -> Vec<super::chat::LlmToolCall> {
    let mut out = Vec::new();
    for (idx, item) in arr.iter().enumerate() {
        let id = item
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("call")
            .to_string();
        let func = item.get("function").or_else(|| item.get("tool"));
        let Some(func) = func else {
            continue;
        };
        let Some(name) = func.get("name").and_then(|v| v.as_str()) else {
            continue;
        };
        let arguments = func
            .get("arguments")
            .map(|a| {
                if let Some(s) = a.as_str() {
                    serde_json::from_str(s).unwrap_or_else(|_| serde_json::json!({ "raw": s }))
                } else {
                    a.clone()
                }
            })
            .unwrap_or_else(|| serde_json::json!({}));
        let id = if id == "call" {
            format!("call_{idx}")
        } else {
            id
        };
        out.push(super::chat::LlmToolCall {
            id,
            name: name.to_string(),
            arguments,
        });
    }
    out
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
                "maxLength": 120,
                "description": "One-line summary"
            },
            "diagnosis": {
                "type": "string",
                "maxLength": 320,
                "description": "Max 2 sentences: what failed, log evidence, merge impact"
            },
            "recommended_action": {
                "type": "string",
                "maxLength": 160,
                "description": "One sentence: concrete next step"
            },
            "test_name": { "type": "string" },
            "page_summary": { "type": "string" }
        },
        "required": ["verdict", "reason", "diagnosis", "recommended_action"],
        "additionalProperties": false
    })
}

/// OpenAI-compatible APIs: JSON mode via `response_format` (portable across DeepSeek, vLLM, OpenAI).
fn apply_openai_json_mode(body: &mut serde_json::Value) {
    let obj = body.as_object_mut().expect("request body object");
    obj.remove("format");
    obj.insert(
        "response_format".into(),
        serde_json::json!({ "type": "json_object" }),
    );
}

fn llm_structured_output_unavailable(err: &crate::error::CoworkerError) -> bool {
    let crate::error::CoworkerError::Other(inner) = err else {
        return false;
    };
    let msg = inner.to_string().to_ascii_lowercase();
    msg.contains("response_format")
        || msg.contains("json_schema")
        || msg.contains("structured outputs")
        || (msg.contains("json_object") && msg.contains("unavailable"))
}

/// Remove Gemma / template control tokens (`<|tool_response|>`, `<channel|>`, etc.).
pub(crate) fn strip_template_tokens(text: &str) -> String {
    static TOKEN: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"<[^>]*\|>").expect("template token regex"));
    let stripped: String = TOKEN.replace_all(text, "").into_owned();
    trim_trailing_template_junk(&stripped)
}

fn trim_trailing_template_junk(s: &str) -> String {
    let mut s = s.trim().to_string();
    while s.ends_with('{') || s.ends_with(',') || s.ends_with(':') || s.ends_with('"') {
        s.pop();
        s = s.trim_end().to_string();
    }
    s
}

/// Strip thinking-mode leaks and mid-field JSON fragments from LLM string fields.
pub(crate) fn sanitize_llm_field(text: &str) -> String {
    let text = strip_template_tokens(text);
    if text.is_empty() {
        return String::new();
    }

    let lower = text.to_ascii_lowercase();
    let mut cut_at = text.len();

    const FENCE_MARKERS: &[&str] = &["```json", "```JSON", "```"];
    const THINKING_MARKERS: &[&str] = &[
        "thought_process:",
        "thoughtful_analysis:",
        "thoughtful_thought",
        "thethought",
        "thoughtly:",
        "orthought:",
        "thought:",
    ];
    const JSON_LEAK_MARKERS: &[&str] = &[" waypoints:", " \"verdict\"", "\n{", "<channel|>", "<|"];

    for marker in FENCE_MARKERS {
        if let Some(idx) = text.find(marker) {
            cut_at = cut_at.min(idx);
        }
    }
    for marker in THINKING_MARKERS {
        if let Some(idx) = lower.find(marker) {
            cut_at = cut_at.min(idx);
        }
    }
    for marker in JSON_LEAK_MARKERS {
        if let Some(idx) = text.find(marker) {
            cut_at = cut_at.min(idx);
        }
    }

    let mut s = text[..cut_at].trim().to_string();
    s = s
        .trim_end_matches(['\'', '"', '`', ',', ' ', ':', '.'])
        .to_string();
    if s.ends_with(" because it") {
        s.truncate(s.len().saturating_sub(" because it".len()));
    }
    s.trim().to_string()
}

/// Strip markdown fences and parse classify JSON from model output.
fn parse_classify_response(
    content: &str,
) -> std::result::Result<ClassifyResponse, serde_json::Error> {
    if let Ok(v) = try_parse_classify_json(content) {
        return Ok(v.sanitized());
    }

    let unfenced = strip_markdown_fence(content.trim());
    if let Ok(v) = try_parse_classify_json(&unfenced) {
        return Ok(v.sanitized());
    }

    if let Some(salvaged) = salvage_truncated_classify(content) {
        return Ok(salvaged.sanitized());
    }

    if let Some(salvaged) = salvage_truncated_classify(&unfenced) {
        return Ok(salvaged.sanitized());
    }

    try_parse_classify_json(content).map(ClassifyResponse::sanitized)
}

fn try_parse_classify_json(
    content: &str,
) -> std::result::Result<ClassifyResponse, serde_json::Error> {
    let trimmed = content.trim();
    if let Ok(v) = serde_json::from_str::<ClassifyResponse>(trimmed) {
        return Ok(v);
    }

    if let Some(json) = extract_json_object(trimmed) {
        if let Ok(v) = serde_json::from_str::<ClassifyResponse>(&json) {
            return Ok(v);
        }
        let repaired = repair_truncated_json_object(&json);
        if let Ok(v) = serde_json::from_str::<ClassifyResponse>(&repaired) {
            return Ok(v);
        }
    }

    let repaired = repair_truncated_json_object(trimmed);
    serde_json::from_str(&repaired)
}

/// Best-effort recovery when the model truncates mid-JSON.
fn salvage_truncated_classify(content: &str) -> Option<ClassifyResponse> {
    let text = extract_json_object(content).unwrap_or_else(|| content.trim().to_string());
    let reason = extract_json_string_field(&text, "reason");
    let diagnosis = extract_json_string_field(&text, "diagnosis");
    let recommended_action = extract_json_string_field(&text, "recommended_action");
    let test_name = extract_json_string_field(&text, "test_name");
    let page_summary = extract_json_string_field(&text, "page_summary");

    if reason.is_none() && diagnosis.is_none() && recommended_action.is_none() {
        return None;
    }

    let verdict = extract_json_string_field(&text, "verdict")
        .as_deref()
        .and_then(parse_verdict_str)
        .unwrap_or(ClassifyVerdict::Unknown);

    Some(ClassifyResponse {
        verdict,
        reason: mark_partial_llm_output(
            reason
                .or(diagnosis.clone())
                .unwrap_or_else(|| "truncated LLM response (partial JSON recovered)".into()),
        ),
        diagnosis,
        recommended_action,
        test_name,
        page_summary,
    })
}

fn mark_partial_llm_output(reason: String) -> String {
    if reason.contains("(partial LLM output)") {
        reason
    } else {
        format!("{reason} (partial LLM output)")
    }
}

fn repair_truncated_json_object(s: &str) -> String {
    let Some(start) = s.find('{') else {
        return s.to_string();
    };
    let mut body = s[start..].trim_end().to_string();

    if unescaped_quote_count(&body) % 2 == 1 {
        body.push('"');
    }

    if !body.contains("\"verdict\"") {
        if body.ends_with('}') {
            body.pop();
            body = body.trim_end().trim_end_matches(',').to_string();
            body.push_str(r#","verdict":"unknown"}"#);
        } else {
            body.push_str(r#","verdict":"unknown"}"#);
        }
    } else {
        let open = body.chars().filter(|&c| c == '{').count();
        let close = body.chars().filter(|&c| c == '}').count();
        for _ in 0..open.saturating_sub(close) {
            body.push('}');
        }
    }

    body
}

fn unescaped_quote_count(s: &str) -> usize {
    let mut count = 0usize;
    let mut escaped = false;
    for ch in s.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '"' {
            count += 1;
        }
    }
    count
}

fn extract_json_string_field(s: &str, key: &str) -> Option<String> {
    let marker = format!("\"{key}\"");
    let key_pos = s.find(&marker)?;
    let mut rest = s[key_pos + marker.len()..].trim_start();
    if !rest.starts_with(':') {
        return None;
    }
    rest = rest[1..].trim_start();
    let rest = rest.strip_prefix('"')?;

    let mut out = String::new();
    let mut chars = rest.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            if let Some(next) = chars.next() {
                match next {
                    'n' => out.push('\n'),
                    't' => out.push('\t'),
                    'r' => out.push('\r'),
                    '"' => out.push('"'),
                    '\\' => out.push('\\'),
                    other => {
                        out.push('\\');
                        out.push(other);
                    }
                }
            } else {
                out.push('\\');
            }
            continue;
        }
        if ch == '"' {
            break;
        }
        out.push(ch);
    }

    if out.is_empty() {
        out = rest.chars().take(400).collect();
    }

    let out = out.trim().to_string();
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

fn parse_verdict_str(s: &str) -> Option<ClassifyVerdict> {
    match s.trim().trim_matches('"') {
        "flaky" => Some(ClassifyVerdict::Flaky),
        "real" => Some(ClassifyVerdict::Real),
        "policy" => Some(ClassifyVerdict::Policy),
        "unknown" => Some(ClassifyVerdict::Unknown),
        _ => None,
    }
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

pub(crate) fn extract_json_object(s: &str) -> Option<String> {
    let start = s.find('{')?;
    if let Some(end) = s.rfind('}') {
        if end > start {
            return Some(s[start..=end].to_string());
        }
    }
    // Truncated JSON — no closing brace; use everything from `{` onward.
    Some(s[start..].to_string())
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
    repo: &str,
    run_id: i64,
    workflow: &str,
    classify: &ClassifyResult,
) -> Vec<String> {
    let verdict = verdict_label(classify.verdict);
    let source = if classify.used_llm {
        "llm"
    } else {
        "heuristic"
    };
    let run_url = github_actions_run_url(repo, run_id);
    let run_ref = format!("[{run_id}]({run_url})");
    let mut lines = vec![format!(
        "- run {run_ref} {workflow} → {verdict} ({} page(s), {source})",
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

/// One-line policy entry for the digest (no Diagnosis/Action blocks).
pub fn format_policy_digest_line(repo: &str, run_id: i64, workflow: &str, hint: &str) -> String {
    let hint = sanitize_llm_field(hint);
    let hint = if hint.is_empty() {
        "resolve policy gate".to_string()
    } else {
        hint.chars().take(100).collect()
    };
    let url = github_actions_run_url(repo, run_id);
    format!("  - [{run_id}]({url}) {workflow} — {hint}")
}

pub fn format_policy_digest_line_from_classify(
    repo: &str,
    run_id: i64,
    workflow: &str,
    classify: &ClassifyResult,
) -> String {
    let hint = classify
        .recommended_action
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .or(classify.diagnosis.as_deref())
        .unwrap_or(classify.reason.trim());
    format_policy_digest_line(repo, run_id, workflow, hint)
}

fn github_actions_run_url(repo: &str, run_id: i64) -> String {
    format!("https://github.com/{repo}/actions/runs/{run_id}")
}

fn verdict_label(v: ClassifyVerdict) -> &'static str {
    match v {
        ClassifyVerdict::Flaky => "flaky",
        ClassifyVerdict::Real => "real bug",
        ClassifyVerdict::Policy => "policy",
        ClassifyVerdict::Unknown => "unknown",
    }
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
    fn thinking_prompt_suffix_when_enabled() {
        let cfg = LlmConfig {
            base_url: "http://localhost:11434/v1".into(),
            model: "gemma4".into(),
            context_limit: 64000,
            log_page_lines: 80,
            max_log_pages: 8,
            concurrency: 2,
            structured_output: true,
            max_output_tokens: 4096,
            think: true,
            max_thinking_tokens: 512,
            reasoning_summary_tokens: 320,
            history_summary_tokens: 256,
            api_key: None,
        };
        let s = thinking_prompt_suffix(&cfg);
        assert!(s.contains("512"));
        assert!(s.contains("128"));
    }

    #[test]
    fn stream_wall_exceeded_reasoning_only_cap() {
        let full = std::time::Duration::from_secs(CHAT_STREAM_WALL_SECS);
        assert!(!stream_wall_exceeded(
            std::time::Duration::from_secs(10),
            Some(std::time::Duration::from_secs(10)),
            0,
            500,
            0,
            full,
            30,
        ));
        assert!(stream_wall_exceeded(
            std::time::Duration::from_secs(10),
            Some(std::time::Duration::from_secs(30)),
            0,
            500,
            0,
            full,
            30,
        ));
        assert!(!stream_wall_exceeded(
            std::time::Duration::from_secs(10),
            Some(std::time::Duration::from_secs(60)),
            12,
            500,
            0,
            full,
            30,
        ));
        assert!(!stream_wall_exceeded(
            std::time::Duration::from_secs(10),
            Some(std::time::Duration::from_secs(60)),
            0,
            500,
            1,
            full,
            30,
        ));
        assert!(stream_wall_exceeded(
            std::time::Duration::from_secs(CHAT_STREAM_WALL_SECS),
            None,
            0,
            0,
            0,
            full,
            30,
        ));
        assert!(!stream_wall_exceeded(
            std::time::Duration::from_secs(60),
            Some(std::time::Duration::from_secs(60)),
            0,
            500,
            0,
            full,
            0,
        ));
    }

    #[test]
    fn append_stream_text_handles_cumulative_chunks() {
        let mut acc = String::from("Wait");
        append_stream_text(&mut acc, "Wait, next PR");
        assert_eq!(acc, "Wait, next PR");
        append_stream_text(&mut acc, " #19258");
        assert_eq!(acc, "Wait, next PR #19258");
    }

    #[test]
    fn stream_text_appears_stuck_on_repeated_tail() {
        let block = "a".repeat(100);
        let stuck = format!("{block}{block}");
        assert!(stream_text_appears_stuck(&stuck));
        assert!(!stream_text_appears_stuck("short"));
    }

    #[test]
    fn extract_openai_plain_content_from_reasoning_only() {
        let v = serde_json::json!({
            "choices": [{
                "finish_reason": "length",
                "message": {
                    "role": "assistant",
                    "content": null,
                    "reasoning_content": "* PR #17671 overview retrieved.\n* Next: list changed files."
                }
            }]
        });
        let text = extract_openai_plain_content(&v).unwrap();
        assert!(text.contains("#17671"));
    }

    #[test]
    fn extract_openai_plain_content_from_tool_calls_only() {
        let v = serde_json::json!({
            "choices": [{
                "finish_reason": "tool_calls",
                "message": {
                    "role": "assistant",
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": {
                            "name": "github:pr_get_diff",
                            "arguments": "{\"file_path\": \"scripts/smart_router.py\"}"
                        }
                    }]
                }
            }]
        });
        let text = extract_openai_plain_content(&v).unwrap();
        assert!(text.contains("pr_get_diff"));
        assert!(text.contains("smart_router.py"));
    }

    #[test]
    fn extract_openai_plain_content_rejects_thought_spam() {
        let v = serde_json::json!({
            "choices": [{
                "finish_reason": "length",
                "message": {
                    "role": "assistant",
                    "content": "<thought\n<thought\n<thought\n<thought",
                    "reasoning_content": "* PR #42: checked CI."
                }
            }]
        });
        let text = extract_openai_plain_content(&v).unwrap();
        assert!(text.contains("#42"));
    }

    #[test]
    fn extract_openai_plain_content_from_reasoning_bullets() {
        let v = serde_json::json!({
            "choices": [{
                "finish_reason": "stop",
                "message": {
                    "content": "",
                    "reasoning_content": "- PR #42: checked CI\n- Next: pr_get_diff"
                }
            }]
        });
        let text = extract_openai_plain_content(&v).unwrap();
        assert!(text.contains("#42"));
        assert!(text.contains("pr_get_diff"));
    }

    #[test]
    fn extract_openai_content_from_reasoning_fallback() {
        let v = serde_json::json!({
            "choices": [{
                "message": {
                    "content": "",
                    "reasoning": "{\"verdict\":\"real\",\"reason\":\"compile error\",\"diagnosis\":\"x\",\"recommended_action\":\"fix\"}"
                }
            }]
        });
        let text = extract_openai_chat_content(&v).unwrap();
        assert!(text.contains("compile error"));
    }

    #[test]
    fn openai_tool_calls_without_content_is_valid() {
        let v = serde_json::json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "tool_calls": [{
                        "id": "call_42d4de21",
                        "type": "function",
                        "function": {
                            "name": "skill_load",
                            "arguments": "{\"name\": \"pr-review\"}"
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }]
        });
        let turn = parse_openai_tools_turn(&v).unwrap();
        assert!(turn.content.trim().is_empty());
        assert_eq!(turn.tool_calls.len(), 1);
        assert_eq!(turn.tool_calls[0].name, "skill_load");
    }

    #[test]
    fn openai_reasoning_content_parsed_separately_from_content() {
        let v = serde_json::json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": "Hi there",
                    "reasoning_content": "Step 1: greet the user.",
                    "tool_calls": []
                },
                "finish_reason": "stop"
            }]
        });
        let msg = v.pointer("/choices/0/message").unwrap();
        assert_eq!(
            extract_openai_message_reasoning(msg),
            "Step 1: greet the user."
        );
        assert_eq!(extract_openai_visible_content(msg), "Hi there");
        let turn = parse_openai_tools_turn(&v).unwrap();
        assert_eq!(turn.reasoning, "Step 1: greet the user.");
        assert_eq!(turn.content, "Hi there");
    }

    #[test]
    fn merge_openai_tool_call_stream_deltas() {
        let mut acc = Vec::new();
        merge_openai_tool_call_deltas(
            &mut acc,
            &[serde_json::json!({
                "index": 0,
                "id": "call_1",
                "function": { "name": "skill_load", "arguments": "{\"na" }
            })],
        );
        merge_openai_tool_call_deltas(
            &mut acc,
            &[serde_json::json!({
                "index": 0,
                "function": { "arguments": "me\": \"pr-review\"}" }
            })],
        );
        let calls = parse_accumulated_openai_tool_calls(&acc);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "skill_load");
        assert_eq!(calls[0].arguments["name"], "pr-review");
    }

    #[test]
    fn openai_chat_completions_url_normalizes_base() {
        assert_eq!(
            openai_chat_completions_url("http://localhost:11434"),
            "http://localhost:11434/v1/chat/completions"
        );
        assert_eq!(
            openai_chat_completions_url("http://localhost:11434/v1"),
            "http://localhost:11434/v1/chat/completions"
        );
    }

    #[test]
    fn openai_json_mode_uses_json_object() {
        let mut body = serde_json::json!({"model": "m"});
        apply_openai_json_mode(&mut body);
        assert_eq!(
            body.pointer("/response_format/type")
                .and_then(|v| v.as_str()),
            Some("json_object")
        );
        assert!(body.get("format").is_none());
    }

    #[test]
    fn structured_output_unavailable_detects_response_format_error() {
        let err = crate::error::CoworkerError::Other(anyhow::anyhow!(
            "llm HTTP 400 Bad Request: This response_format type is unavailable now"
        ));
        assert!(llm_structured_output_unavailable(&err));
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
    fn extract_json_object_handles_truncated() {
        let raw = r#"{"reason":"x","action":"y"#;
        let obj = extract_json_object(raw).unwrap();
        assert_eq!(obj, raw);
    }

    #[test]
    fn sanitize_llm_field_strips_thinking_json_leaks() {
        let raw = "The PR requires manager approval. The workflow failed because it'```json waypoints: [] { ";
        let clean = sanitize_llm_field(raw);
        assert!(!clean.contains("```json"));
        assert!(!clean.contains("waypoints"));
        assert!(clean.contains("manager approval"));
    }

    #[test]
    fn sanitize_llm_field_strips_channel_token() {
        let raw = "* **PR Overview:** snapshots\n<channel|>{";
        let clean = sanitize_llm_field(raw);
        assert!(clean.contains("PR Overview"));
        assert!(!clean.contains("<channel|>"));
        assert!(!clean.ends_with('{'));
    }

    #[test]
    fn sanitize_llm_field_strips_thought_process_leak() {
        let raw =
            "Missing approval for this PR, or thethought_process: The user wants me to triage";
        let clean = sanitize_llm_field(raw);
        assert!(!clean.contains("thought_process"));
        assert!(clean.starts_with("Missing approval"));
    }

    #[test]
    fn parse_classify_salvages_truncated_json() {
        let raw = r#"{
  "diagnosis": "The backport checker failed because the PR lacks required manager approval.",
  "reason": "Backport approval missing",
  "recommended_action": "The author should either add the approval label or apply the
"#;
        let r = parse_classify_response(raw).unwrap();
        assert_eq!(r.verdict, ClassifyVerdict::Unknown);
        assert!(r
            .diagnosis
            .as_ref()
            .is_some_and(|d| d.contains("backport checker")));
        assert!(r
            .recommended_action
            .as_ref()
            .is_some_and(|a| a.contains("author should")));
        assert!(r.reason.contains("Backport approval missing"));
    }

    #[test]
    fn format_policy_digest_line_is_compact() {
        let c = ClassifyResult {
            verdict: ClassifyVerdict::Policy,
            reason: "approval missing".into(),
            diagnosis: Some("Long diagnosis that should not appear as separate block.".into()),
            recommended_action: Some("Obtain manager approval.".into()),
            test_name: Some("N/A".into()),
            used_llm: true,
            pages_read: 1,
            page_summary: None,
        };
        let line = format_policy_digest_line_from_classify(
            "acme/widget",
            27400805815,
            "Backport PR manager approval checker",
            &c,
        );
        assert!(
            line.contains("[27400805815](https://github.com/acme/widget/actions/runs/27400805815)")
        );
        assert!(line.contains("Obtain manager approval"));
        assert!(!line.contains("Diagnosis:"));
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
        let lines = format_classify_digest_lines("org/repo", 123, "Backport checker", &c);
        assert!(lines[0].contains("[123](https://github.com/org/repo/actions/runs/123)"));
        assert!(lines.iter().any(|l| l.contains("Diagnosis:")));
        assert!(lines.iter().any(|l| l.contains("Action:")));
    }

    #[test]
    fn digest_run_link_for_all_verdicts() {
        let c = ClassifyResult {
            verdict: ClassifyVerdict::Flaky,
            reason: "network timeout".into(),
            diagnosis: Some("Bazel fetch timed out.".into()),
            recommended_action: Some("Rerun workflow.".into()),
            test_name: None,
            used_llm: true,
            pages_read: 1,
            page_summary: None,
        };
        let lines =
            format_classify_digest_lines("acme/widget", 27400326361, "Package & Release", &c);
        assert!(lines[0]
            .contains("[27400326361](https://github.com/acme/widget/actions/runs/27400326361)"));

        let policy = ClassifyResult {
            verdict: ClassifyVerdict::Policy,
            reason: "approval".into(),
            diagnosis: None,
            recommended_action: None,
            test_name: None,
            used_llm: true,
            pages_read: 1,
            page_summary: None,
        };
        let policy_lines =
            format_classify_digest_lines("acme/widget", 27400805815, "Backport checker", &policy);
        assert!(policy_lines[0]
            .contains("[27400805815](https://github.com/acme/widget/actions/runs/27400805815)"));
    }
}
