use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{CoworkerError, Result};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    pub llm: LlmConfig,
    pub mcp: McpConfig,
    pub storage: StorageConfig,
    #[serde(default)]
    pub schedule: ScheduleConfig,
    #[serde(default)]
    pub workflows: HashMap<String, WorkflowConfig>,
    pub repos: Vec<String>,
    #[serde(default)]
    pub policy: PolicyConfig,
    #[serde(default)]
    pub flaky: FlakyConfig,
    #[serde(default)]
    pub output: OutputConfig,
    #[serde(default)]
    pub release: ReleaseConfig,
    #[serde(default)]
    pub main_guard: MainGuardConfig,
    #[serde(default)]
    pub chat: ChatConfig,
    #[serde(default)]
    pub tui: TuiConfig,
    #[serde(default)]
    pub rules: Vec<RuleConfig>,
    #[serde(default)]
    pub hygiene: HygieneConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LlmConfig {
    pub base_url: String,
    pub model: String,
    pub context_limit: u32,
    /// Lines per page when fetching/analyzing CI logs (`ci_get_failed_logs max_lines`).
    #[serde(default = "default_log_page_lines")]
    pub log_page_lines: u32,
    /// Max log pages to fetch + analyze per failing run before giving up.
    #[serde(default = "default_max_log_pages")]
    pub max_log_pages: u32,
    /// Max concurrent in-flight requests to the LLM (Ollama typically handles ~2).
    #[serde(default = "default_llm_concurrency")]
    pub concurrency: u32,
    /// Constrain LLM replies to a JSON schema (Ollama structured outputs / OpenAI json_schema).
    #[serde(default = "default_structured_output")]
    pub structured_output: bool,
    /// Max tokens for classify output (Ollama `num_predict` / OpenAI `max_tokens`).
    /// Includes both reasoning trace and final JSON on thinking models.
    #[serde(default = "default_max_output_tokens")]
    pub max_output_tokens: u32,
    /// Enable model reasoning (Ollama top-level `think`). Default on for gemma4/qwen3.
    #[serde(default = "default_llm_think")]
    pub think: bool,
    /// Soft cap on reasoning length — enforced via prompt; logged when exceeded.
    #[serde(default = "default_max_thinking_tokens")]
    pub max_thinking_tokens: u32,
    /// Max tokens for think=false reasoning compression calls (chat context).
    #[serde(default = "default_reasoning_summary_tokens")]
    pub reasoning_summary_tokens: u32,
    /// Max tokens for think=false session history rolling summary.
    #[serde(default = "default_history_summary_tokens")]
    pub history_summary_tokens: u32,
}

fn default_history_summary_tokens() -> u32 {
    256
}

fn default_reasoning_summary_tokens() -> u32 {
    320
}

fn default_max_output_tokens() -> u32 {
    4096
}

fn default_llm_think() -> bool {
    true
}

fn default_max_thinking_tokens() -> u32 {
    512
}

fn default_structured_output() -> bool {
    true
}

fn default_llm_concurrency() -> u32 {
    2
}

fn default_log_page_lines() -> u32 {
    80
}

fn default_max_log_pages() -> u32 {
    8
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct McpConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum StorageBackend {
    Json,
    Sqlite,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StorageConfig {
    pub backend: StorageBackend,
    pub path: String,
    #[serde(default)]
    pub wal: bool,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ScheduleConfig {
    pub daily_digest: Option<String>,
    pub ci_rescan: Option<String>,
    pub main_guard: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WorkflowConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Task spec markdown (agents/*/AGENT.md).
    #[serde(default)]
    pub agent: Option<String>,
    /// Technique skills to compose into prompts.
    #[serde(default)]
    pub skills: Vec<String>,
    pub schedule: Option<String>,
    #[serde(default)]
    pub mutating: Vec<String>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct PolicyConfig {
    #[serde(default)]
    pub auto_rerun_flaky: bool,
    #[serde(default)]
    pub auto_backport: bool,
    #[serde(default = "default_max_prs")]
    pub max_prs_per_repo: u32,
    #[serde(default = "default_max_turns")]
    pub max_agent_turns: u32,
    #[serde(default = "default_max_tool_calls")]
    pub max_tool_calls_per_pr: u32,
}

fn default_max_prs() -> u32 {
    20
}
fn default_max_turns() -> u32 {
    12
}
fn default_max_tool_calls() -> u32 {
    5
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct FlakyConfig {
    #[serde(default = "default_true")]
    pub record_real_bugs: bool,
    #[serde(default = "default_fingerprint_fallback")]
    pub fingerprint_fallback: String,
}

fn default_fingerprint_fallback() -> String {
    "error".into()
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct OutputConfig {
    #[serde(default)]
    pub export_digest_md: bool,
    #[serde(default = "default_digest_path")]
    pub digest_export_path: String,
    /// Optional Slack incoming webhook URL for digest summaries (headless / daemon).
    #[serde(default)]
    pub slack_webhook: Option<String>,
}

fn default_digest_path() -> String {
    "./digests/{date}.md".into()
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ReleaseConfig {
    #[serde(default = "default_backport_label")]
    pub backport_label: String,
    #[serde(default)]
    pub target_branches: Vec<String>,
    #[serde(default = "default_lookback_limit")]
    pub lookback_limit: u32,
}

fn default_backport_label() -> String {
    "needs-backport".into()
}

fn default_lookback_limit() -> u32 {
    30
}

impl Default for ReleaseConfig {
    fn default() -> Self {
        Self {
            backport_label: default_backport_label(),
            target_branches: vec![],
            lookback_limit: default_lookback_limit(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MainGuardConfig {
    #[serde(default = "default_consecutive_failures")]
    pub consecutive_failures: u32,
    #[serde(default = "default_recent_runs")]
    pub recent_runs: u32,
}

fn default_consecutive_failures() -> u32 {
    2
}

fn default_recent_runs() -> u32 {
    15
}

impl Default for MainGuardConfig {
    fn default() -> Self {
        Self {
            consecutive_failures: default_consecutive_failures(),
            recent_runs: default_recent_runs(),
        }
    }
}

fn default_chat_agent() -> String {
    "agents/chat/AGENT.md".into()
}

fn default_chat_skills() -> Vec<String> {
    vec![
        "skills/github-ops-tone/SKILL.md".into(),
        "skills/ci-triage/SKILL.md".into(),
    ]
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChatConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_chat_agent")]
    pub agent: String,
    #[serde(default = "default_chat_skills")]
    pub skills: Vec<String>,
    #[serde(default = "default_chat_max_turns")]
    pub max_turns: u32,
    #[serde(default = "default_chat_max_tool_calls")]
    pub max_tool_calls: u32,
    /// Wall-clock seconds for one user message (LLM + tools). 0 = unlimited.
    #[serde(default = "default_chat_max_duration_secs")]
    pub max_duration_secs: u64,
    /// Max seconds for a single LLM step (streaming thinking + JSON). 0 = unlimited.
    #[serde(default = "default_chat_llm_step_timeout_secs")]
    pub llm_step_timeout_secs: u64,
    #[serde(default = "default_chat_history_messages")]
    pub history_messages: u32,
    /// Max tokens for prior session turns in LLM context. 0 = auto (~40% of input budget).
    #[serde(default)]
    pub history_tokens: u32,
    /// Read-only tools pre-registered for chat (MCP names + coworker virtual tools).
    #[serde(default = "default_chat_preferred_tools")]
    pub preferred_tools: Vec<String>,
    /// Summarize Ollama thinking via a fast think=false LLM call for context + TUI.
    #[serde(default = "default_true")]
    pub compress_reasoning: bool,
    /// Min thinking chars before triggering LLM compression.
    #[serde(default = "default_reasoning_compress_min_chars")]
    pub reasoning_compress_min_chars: u32,
    /// LLM rolling summary for session history when it exceeds the token budget.
    #[serde(default = "default_true")]
    pub compress_history: bool,
    /// Min estimated tokens in dropped history before LLM summary (else local omit).
    #[serde(default = "default_history_summary_min_tokens")]
    pub history_summary_min_tokens: u32,
    /// When true, mutating chat tools (rerun, backport, comment) run immediately without a popup.
    #[serde(default)]
    pub auto_approve_mutations: bool,
}

fn default_history_summary_min_tokens() -> u32 {
    400
}

fn default_reasoning_compress_min_chars() -> u32 {
    480
}

fn default_chat_max_turns() -> u32 {
    0
}

fn default_chat_max_tool_calls() -> u32 {
    0
}

fn default_chat_max_duration_secs() -> u64 {
    900
}

fn default_chat_llm_step_timeout_secs() -> u64 {
    180
}

fn default_chat_history_messages() -> u32 {
    24
}

pub fn default_chat_preferred_tools() -> Vec<String> {
    vec![
        "pr_get_overview".into(),
        "pr_list_changed_files".into(),
        "pr_get_diff".into(),
        "pr_list_open".into(),
        "pr_list_waiting_review".into(),
        "pr_get_merge_blockers".into(),
        "pr_get_status".into(),
        "pr_list_merged".into(),
        "ci_analyze_pr_failures".into(),
        "ci_get_run_summary".into(),
        "ci_get_failed_logs".into(),
        "issue_list_open".into(),
        "issue_get".into(),
        "alert_list_open".into(),
        "store_get_latest_digest".into(),
    ]
}

impl Default for ChatConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            agent: default_chat_agent(),
            skills: default_chat_skills(),
            max_turns: default_chat_max_turns(),
            max_tool_calls: default_chat_max_tool_calls(),
            max_duration_secs: default_chat_max_duration_secs(),
            llm_step_timeout_secs: default_chat_llm_step_timeout_secs(),
            history_messages: default_chat_history_messages(),
            history_tokens: 0,
            preferred_tools: default_chat_preferred_tools(),
            compress_reasoning: true,
            reasoning_compress_min_chars: default_reasoning_compress_min_chars(),
            compress_history: true,
            history_summary_min_tokens: default_history_summary_min_tokens(),
            auto_approve_mutations: false,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TuiThemeMode {
    #[default]
    Dark,
    Light,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TuiConfig {
    #[serde(default)]
    pub theme: TuiThemeMode,
}

impl Default for TuiConfig {
    fn default() -> Self {
        Self {
            theme: TuiThemeMode::Dark,
        }
    }
}

/// YAML rule: if workflow/error match, suggest action before LLM classify.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RuleConfig {
    #[serde(default)]
    pub workflow: Option<String>,
    /// Substring match on error log text (case-insensitive).
    #[serde(default, rename = "error~")]
    pub error_contains: Option<String>,
    #[serde(default = "default_rule_action")]
    pub then: RuleAction,
}

fn default_rule_action() -> RuleAction {
    RuleAction::SuggestRerun
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuleAction {
    SuggestRerun,
    MarkFlaky,
    SkipLlm,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HygieneConfig {
    #[serde(default = "default_stale_days")]
    pub stale_days: u32,
    #[serde(default = "default_large_pr_lines")]
    pub large_pr_lines: u32,
}

fn default_stale_days() -> u32 {
    7
}

fn default_large_pr_lines() -> u32 {
    500
}

impl Default for HygieneConfig {
    fn default() -> Self {
        Self {
            stale_days: default_stale_days(),
            large_pr_lines: default_large_pr_lines(),
        }
    }
}

impl Config {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let raw = std::fs::read_to_string(path.as_ref())?;
        let mut cfg: Config = serde_yaml::from_str(&raw)?;
        cfg.resolve_env_in_mcp();
        Ok(cfg)
    }

    pub fn discover() -> Result<(Self, PathBuf)> {
        let candidates = [
            PathBuf::from("coworker.yaml"),
            PathBuf::from(".coworker/coworker.yaml"),
        ];
        for path in candidates {
            if path.exists() {
                return Ok((Self::load(&path)?, path));
            }
        }
        Err(CoworkerError::Config(
            "coworker.yaml not found (cwd or .coworker/) — copy coworker.example.yaml to coworker.yaml".into(),
        ))
    }

    pub fn storage_path(&self) -> PathBuf {
        expand_tilde(&self.storage.path)
    }

    fn resolve_env_in_mcp(&mut self) {
        for value in self.mcp.env.values_mut() {
            if let Some(rest) = value.strip_prefix("${") {
                if let Some(var) = rest.strip_suffix('}') {
                    if let Ok(v) = std::env::var(var) {
                        *value = v;
                    }
                }
            }
        }
    }
}

pub fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_home() {
        let p = expand_tilde("~/foo");
        assert!(p.to_string_lossy().contains("foo"));
    }
}
