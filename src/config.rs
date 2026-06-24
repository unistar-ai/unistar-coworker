use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{CoworkerError, Result};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    pub llm: LlmConfig,
    #[serde(default)]
    pub github: GithubConfig,
    #[serde(default)]
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
    /// UI theme for TUI and Web (`dark`, `light`, or `none` for terminal defaults in TUI only).
    #[serde(default)]
    pub theme: Option<ThemeMode>,
    #[serde(default)]
    pub tui: TuiConfig,
    #[serde(default)]
    pub web: WebConfig,
    #[serde(default)]
    pub rules: Vec<RuleConfig>,
    #[serde(default)]
    pub hygiene: HygieneConfig,
    #[serde(default)]
    pub mcp: McpConfig,
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
    /// Bearer token for OpenAI-compatible servers (oMLX, vLLM, etc.). Optional for Ollama.
    #[serde(default)]
    pub api_key: Option<String>,
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
pub struct GithubConfig {
    /// GitHub CLI binary (default `gh`).
    #[serde(default = "default_gh_command")]
    pub gh_command: String,
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// Default tool RPC timeout in seconds (default 120).
    #[serde(default = "default_mcp_timeout_secs")]
    pub timeout_secs: u64,
    #[serde(default)]
    pub tool_timeouts: HashMap<String, u64>,
}

fn default_gh_command() -> String {
    "gh".into()
}

fn default_mcp_timeout_secs() -> u64 {
    120
}

/// Third-party MCP server federation (`mcp.servers[]`).
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct McpConfig {
    #[serde(default)]
    pub defaults: McpDefaults,
    #[serde(default)]
    pub servers: Vec<McpServerConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct McpDefaults {
    #[serde(default = "default_mcp_timeout_secs")]
    pub timeout_secs: u64,
    #[serde(default = "default_true")]
    pub lazy: bool,
    #[serde(default)]
    pub startup: McpStartup,
    #[serde(default = "default_mcp_max_output_chars")]
    pub max_output_chars: usize,
}

fn default_mcp_max_output_chars() -> usize {
    24_000
}

impl Default for McpDefaults {
    fn default() -> Self {
        Self {
            timeout_secs: default_mcp_timeout_secs(),
            lazy: true,
            startup: McpStartup::OnDemand,
            max_output_chars: default_mcp_max_output_chars(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum McpStartup {
    #[default]
    OnDemand,
    Eager,
    Disabled,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct McpServerConfig {
    pub id: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub transport: McpTransport,
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    pub url: Option<String>,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(default)]
    pub expose: McpExposeConfig,
    #[serde(default)]
    pub approval: McpApprovalConfig,
    #[serde(default)]
    pub startup: Option<McpStartup>,
    /// Per-server RPC timeout; falls back to `mcp.defaults.timeout_secs`.
    #[serde(default)]
    pub timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum McpTransport {
    #[default]
    Stdio,
    Http,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct McpExposeConfig {
    pub prefix: Option<String>,
    #[serde(default)]
    pub allowlist: Vec<String>,
    #[serde(default)]
    pub denylist: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct McpApprovalConfig {
    #[serde(default)]
    pub mutating: McpMutatingPolicy,
    #[serde(default)]
    pub tools: Vec<String>,
}

impl Default for McpApprovalConfig {
    fn default() -> Self {
        Self {
            mutating: McpMutatingPolicy::Deny,
            tools: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum McpMutatingPolicy {
    #[default]
    Deny,
    Required,
    Auto,
}

impl Default for GithubConfig {
    fn default() -> Self {
        Self {
            gh_command: default_gh_command(),
            env: HashMap::new(),
            timeout_secs: default_mcp_timeout_secs(),
            tool_timeouts: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum StorageBackend {
    #[default]
    Json,
    Sqlite,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StorageConfig {
    #[serde(default)]
    pub backend: StorageBackend,
    #[serde(default = "default_storage_path")]
    pub path: String,
    #[serde(default)]
    pub wal: bool,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            backend: StorageBackend::Json,
            path: default_storage_path(),
            wal: false,
        }
    }
}

fn default_storage_path() -> String {
    "./data".into()
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ScheduleConfig {
    #[serde(default = "default_daily_digest_cron")]
    pub daily_digest: Option<String>,
    #[serde(default = "default_ci_rescan_cron")]
    pub ci_rescan: Option<String>,
    pub main_guard: Option<String>,
}

impl Default for ScheduleConfig {
    fn default() -> Self {
        Self {
            daily_digest: default_daily_digest_cron(),
            ci_rescan: default_ci_rescan_cron(),
            main_guard: None,
        }
    }
}

fn default_daily_digest_cron() -> Option<String> {
    Some("0 6 * * *".into())
}

fn default_ci_rescan_cron() -> Option<String> {
    Some("0 */4 * * *".into())
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WorkflowConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Technique skills; default from built-in workflow registry.
    #[serde(default)]
    pub skills: Vec<String>,
    pub schedule: Option<String>,
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
    #[serde(default = "default_max_tool_calls")]
    pub max_tool_calls_per_pr: u32,
}

fn default_max_prs() -> u32 {
    20
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

fn default_chat_prompt() -> String {
    "prompts/chat.md".into()
}

fn default_chat_skills() -> Vec<String> {
    // Empty — use `prompts/chat.md` frontmatter `skills:` as SSOT.
    Vec::new()
}

fn default_chat_workspace() -> PathBuf {
    PathBuf::from(".")
}

/// Session history / tool-result compaction for chat (`code` = coding-first default).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatCompaction {
    /// Preserve paths, errors, edit targets (coding chat default).
    #[default]
    Code,
    /// Preserve CI_KIND, verdicts, PR refs, digest excerpts (ops / MCP triage).
    Ops,
    /// Generic LLM rolling summary without domain-specific keep rules.
    Generic,
}

impl ChatCompaction {
    pub fn to_strategy(self) -> crate::agent::context::CompactionStrategy {
        match self {
            Self::Code => crate::agent::context::CompactionStrategy::Code,
            Self::Ops => crate::agent::context::CompactionStrategy::Ops,
            Self::Generic => crate::agent::context::CompactionStrategy::Generic,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChatConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Local coding workspace root (paths in file tools must resolve under this).
    #[serde(default = "default_chat_workspace")]
    pub workspace: PathBuf,
    #[serde(default = "default_chat_prompt", alias = "agent")]
    pub prompt: String,
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
    /// How chat exposes tools to the LLM: lazy discovery vs full native schemas.
    #[serde(default)]
    pub tool_mode: ChatToolMode,
    /// Whitelisted local shell (`bash_run`).
    #[serde(default)]
    pub bash: BashToolConfig,
    /// Run Python snippets (`python_run`).
    #[serde(default)]
    pub python: PythonToolConfig,
    /// Read-only web fetch (`web_fetch`).
    #[serde(default, alias = "web_browser")]
    pub web_fetch: WebFetchToolConfig,
    /// How older turns and tool output are compressed when context is tight.
    #[serde(default)]
    pub compaction: ChatCompaction,
}

/// `bash_run` — LLM-reviewed local shell (`timeout_secs`, `max_output_chars`).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BashToolConfig {
    #[serde(default = "default_bash_timeout_secs")]
    pub timeout_secs: u64,
    #[serde(default = "default_bash_max_output_chars")]
    pub max_output_chars: usize,
}

impl Default for BashToolConfig {
    fn default() -> Self {
        Self {
            timeout_secs: default_bash_timeout_secs(),
            max_output_chars: default_bash_max_output_chars(),
        }
    }
}

fn default_bash_timeout_secs() -> u64 {
    30
}

fn default_bash_max_output_chars() -> usize {
    16_000
}

/// `python_run` — execute Python in the workspace (`timeout_secs`, `max_output_chars`, `command`).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PythonToolConfig {
    #[serde(default = "default_python_timeout_secs")]
    pub timeout_secs: u64,
    #[serde(default = "default_python_max_output_chars")]
    pub max_output_chars: usize,
    /// Interpreter binary (default `python3`).
    #[serde(default = "default_python_command")]
    pub command: String,
}

impl Default for PythonToolConfig {
    fn default() -> Self {
        Self {
            timeout_secs: default_python_timeout_secs(),
            max_output_chars: default_python_max_output_chars(),
            command: default_python_command(),
        }
    }
}

fn default_python_timeout_secs() -> u64 {
    30
}

fn default_python_max_output_chars() -> usize {
    16_000
}

fn default_python_command() -> String {
    "python3".into()
}

/// `web_fetch` — fetch page text for the agent (`timeout_secs`, charset, SSRF, cache).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WebFetchToolConfig {
    #[serde(default = "default_web_browser_timeout_secs")]
    pub timeout_secs: u64,
    #[serde(default = "default_web_browser_max_content_chars")]
    pub max_content_chars: usize,
    #[serde(default = "default_web_browser_max_download_bytes")]
    pub max_download_bytes: usize,
    #[serde(default = "default_web_browser_user_agent")]
    pub user_agent: String,
    #[serde(default)]
    pub allow_localhost: bool,
    #[serde(default = "default_web_browser_cache_ttl_secs")]
    pub cache_ttl_secs: u64,
    #[serde(default = "default_web_browser_spa_empty_chars")]
    pub spa_empty_chars: usize,
    #[serde(default = "default_web_browser_max_links")]
    pub max_links: usize,
    #[serde(default = "default_web_browser_browser_timeout_secs")]
    pub browser_timeout_secs: u64,
    /// Extra wait after navigation for JS challenges (e.g. zse-ck) before reading the DOM.
    #[serde(default = "default_web_browser_browser_wait_ms")]
    pub browser_wait_ms: u64,
    /// Optional path to Chrome/Chromium binary (otherwise PATH).
    #[serde(default)]
    pub chromium_path: Option<String>,
}

impl Default for WebFetchToolConfig {
    fn default() -> Self {
        Self {
            timeout_secs: default_web_browser_timeout_secs(),
            max_content_chars: default_web_browser_max_content_chars(),
            max_download_bytes: default_web_browser_max_download_bytes(),
            user_agent: default_web_browser_user_agent(),
            allow_localhost: false,
            cache_ttl_secs: default_web_browser_cache_ttl_secs(),
            spa_empty_chars: default_web_browser_spa_empty_chars(),
            max_links: default_web_browser_max_links(),
            browser_timeout_secs: default_web_browser_browser_timeout_secs(),
            browser_wait_ms: default_web_browser_browser_wait_ms(),
            chromium_path: None,
        }
    }
}

fn default_web_browser_browser_timeout_secs() -> u64 {
    60
}

fn default_web_browser_browser_wait_ms() -> u64 {
    3_000
}

fn default_web_browser_timeout_secs() -> u64 {
    30
}

fn default_web_browser_max_content_chars() -> usize {
    32_000
}

fn default_web_browser_max_download_bytes() -> usize {
    2 * 1024 * 1024
}

fn default_web_browser_user_agent() -> String {
    "unistar-coworker/1.0 (+local coding agent)".into()
}

fn default_web_browser_cache_ttl_secs() -> u64 {
    60
}

fn default_web_browser_spa_empty_chars() -> usize {
    80
}

fn default_web_browser_max_links() -> usize {
    20
}

/// Chat tool exposure: progressive discovery (default), legacy lazy alias, or full native schemas.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatToolMode {
    /// Lazy discovery + session warmup + intent hints (recommended).
    #[default]
    Auto,
    /// Same as `auto` (kept for compatibility).
    Lazy,
    /// Full catalog native schemas.
    Native,
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

impl Default for ChatConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            workspace: default_chat_workspace(),
            prompt: default_chat_prompt(),
            skills: default_chat_skills(),
            max_turns: default_chat_max_turns(),
            max_tool_calls: default_chat_max_tool_calls(),
            max_duration_secs: default_chat_max_duration_secs(),
            llm_step_timeout_secs: default_chat_llm_step_timeout_secs(),
            history_messages: default_chat_history_messages(),
            history_tokens: 0,
            compress_reasoning: true,
            reasoning_compress_min_chars: default_reasoning_compress_min_chars(),
            compress_history: true,
            history_summary_min_tokens: default_history_summary_min_tokens(),
            auto_approve_mutations: false,
            tool_mode: ChatToolMode::Auto,
            bash: BashToolConfig::default(),
            python: PythonToolConfig::default(),
            web_fetch: WebFetchToolConfig::default(),
            compaction: ChatCompaction::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeMode {
    #[default]
    Dark,
    Light,
    /// Terminal default colors — TUI only; Web UI uses `dark`.
    None,
}

/// Back-compat alias for docs / external references.
#[allow(dead_code)]
pub type TuiThemeMode = ThemeMode;

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct TuiConfig {
    /// Deprecated: use top-level `theme`.
    #[serde(default, skip_serializing)]
    pub theme: Option<ThemeMode>,
    /// Emit OSC 8 hyperlinks for markdown links (iTerm/WezTerm/Windows Terminal).
    #[serde(default)]
    pub osc8_links: bool,
    /// Optional `#RRGGBB` accent for dark/light themes (ignored when `theme: none`).
    #[serde(default)]
    pub accent: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WebConfig {
    /// Bind address for `unistar-coworker serve` (default 127.0.0.1:8787).
    #[serde(default = "default_web_bind")]
    pub bind: String,
}

fn default_web_bind() -> String {
    "127.0.0.1:8787".into()
}

impl Default for WebConfig {
    fn default() -> Self {
        Self {
            bind: default_web_bind(),
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
        cfg.resolve_env_in_github();
        cfg.resolve_env_in_mcp();
        cfg.finalize();
        Ok(cfg)
    }

    #[cfg(test)]
    pub(crate) fn load_from_str(raw: &str) -> Result<Self> {
        let mut cfg: Config = serde_yaml::from_str(raw)?;
        cfg.resolve_env_in_github();
        cfg.resolve_env_in_mcp();
        Ok(cfg)
    }

    /// Reserved for derived fields after YAML deserialization.
    pub fn finalize(&mut self) {
        self.chat.workspace = expand_tilde(&self.chat.workspace.to_string_lossy());
        if let Ok(canonical) = self.chat.workspace.canonicalize() {
            self.chat.workspace = canonical;
        }
        if self.theme.is_some() && self.tui.theme.is_some() {
            tracing::warn!(
                "coworker.yaml sets both `theme` and `tui.theme` — using top-level `theme`"
            );
        }
    }

    /// Resolved UI theme (`theme` overrides deprecated `tui.theme`).
    pub fn theme(&self) -> ThemeMode {
        self.theme.or(self.tui.theme).unwrap_or_default()
    }

    /// Web UI `data-theme` id (`none` maps to `dark`).
    pub fn web_theme_id(&self) -> &'static str {
        match self.theme() {
            ThemeMode::Light => "light",
            ThemeMode::Dark | ThemeMode::None => "dark",
        }
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

    fn resolve_env_in_github(&mut self) {
        resolve_env_map(&mut self.github.env);
    }

    fn resolve_env_in_mcp(&mut self) {
        for server in &mut self.mcp.servers {
            resolve_env_map(&mut server.env);
            resolve_env_map(&mut server.headers);
        }
    }
}

fn resolve_env_map(map: &mut HashMap<String, String>) {
    for value in map.values_mut() {
        if let Some(rest) = value.strip_prefix("${") {
            if let Some(var) = rest.strip_suffix('}') {
                if let Ok(v) = std::env::var(var) {
                    *value = v;
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

    #[test]
    fn chat_github_probe_tools_in_catalog() {
        use crate::agent::tool_catalog::ToolCatalog;
        use crate::github::helpers::chat_github_probe_tools;

        let cat = ToolCatalog::new();
        for tool in chat_github_probe_tools() {
            assert!(
                cat.is_known_chat_tool(tool),
                "CHAT_GITHUB_TOOLS probe missing from catalog: {tool}"
            );
        }
    }

    #[test]
    fn root_theme_deserializes() {
        let yaml = r#"
llm: { base_url: http://localhost:11434/v1, model: m, context_limit: 64000 }
storage: { backend: json, path: ./data }
repos: [acme/widget]
theme: light
"#;
        let cfg: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(cfg.theme(), ThemeMode::Light);
        assert_eq!(cfg.web_theme_id(), "light");
    }

    #[test]
    fn tui_theme_none_deserializes() {
        let yaml = r#"
llm: { base_url: http://localhost:11434/v1, model: m, context_limit: 64000 }
storage: { backend: json, path: ./data }
repos: [acme/widget]
tui:
  theme: none
"#;
        let cfg: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(cfg.theme(), ThemeMode::None);
    }

    #[test]
    fn tui_accent_hex_deserializes() {
        let yaml = r#"
llm: { base_url: http://localhost:11434/v1, model: m, context_limit: 64000 }
storage: { backend: json, path: ./data }
repos: [acme/widget]
tui:
  accent: '#aabbcc'
"#;
        let cfg: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(cfg.tui.accent.as_deref(), Some("#aabbcc"));
    }

    #[test]
    fn chat_tool_mode_deserializes() {
        let yaml = r#"
llm: { base_url: http://localhost:11434/v1, model: m, context_limit: 64000 }
storage: { backend: json, path: ./data }
repos: [acme/widget]
chat:
  tool_mode: native
"#;
        let cfg: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(cfg.chat.tool_mode, ChatToolMode::Native);
    }

    #[test]
    fn chat_tool_mode_defaults_lazy() {
        let yaml = r#"
llm: { base_url: http://localhost:11434/v1, model: m, context_limit: 64000 }
repos: [acme/widget]
"#;
        let cfg: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(cfg.chat.tool_mode, ChatToolMode::Auto);
    }

    #[test]
    fn minimal_yaml_parses() {
        let yaml = r#"
llm:
  base_url: http://localhost:11434/v1
  model: m
  context_limit: 64000
repos:
  - acme/widget
workflows:
  daily-work: {}
"#;
        let mut cfg = Config::load_from_str(yaml).unwrap();
        cfg.finalize();
        assert_eq!(cfg.github.gh_command, "gh");
        assert!(cfg.workflows.get("daily-work").unwrap().enabled);
        assert_eq!(cfg.storage.backend, StorageBackend::Json);
        assert_eq!(cfg.storage.path, "./data");
        assert!(cfg.schedule.daily_digest.is_some());
    }

    #[test]
    fn mcp_servers_parse() {
        let yaml = r#"
llm: { base_url: http://localhost:11434/v1, model: m, context_limit: 64000 }
repos: [org/repo]
mcp:
  servers:
    - id: slack
      transport: stdio
      command: npx
      args: ["-y", "@modelcontextprotocol/server-slack"]
      env:
        SLACK_BOT_TOKEN: ${SLACK_BOT_TOKEN}
      expose:
        prefix: slack_
        allowlist: [post_message]
    - id: ops
      transport: http
      url: http://127.0.0.1:9090/mcp
      headers:
        Authorization: Bearer ${OPS_MCP_TOKEN}
"#;
        let cfg = Config::load_from_str(yaml).expect("parse");
        assert_eq!(cfg.mcp.servers.len(), 2);
        assert_eq!(cfg.mcp.servers[1].id, "ops");
        assert_eq!(cfg.mcp.servers[1].transport, McpTransport::Http);
        assert_eq!(
            cfg.mcp.servers[1].url.as_deref(),
            Some("http://127.0.0.1:9090/mcp")
        );
    }
}
