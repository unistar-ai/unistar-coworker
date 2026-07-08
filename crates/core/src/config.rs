use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::de::Error as DeError;
use serde::{Deserialize, Deserializer, Serialize};
use serde_yaml::Value;

use crate::error::{CoworkerError, Result};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    /// Active resolved LLM — populated in `finalize()` from `llm` + `llm_profile`.
    #[serde(skip)]
    pub llm: LlmConfig,
    /// Active preset name under YAML `llm`.
    #[serde(default)]
    pub llm_profile: Option<String>,
    /// Named LLM endpoints (YAML key `llm`).
    #[serde(default, rename = "llm", deserialize_with = "deserialize_llm_profiles")]
    pub llm_profiles: HashMap<String, LlmConfig>,
    #[serde(default)]
    pub github: GithubConfig,
    #[serde(default)]
    pub storage: StorageConfig,
    #[serde(default)]
    pub schedule: ScheduleConfig,
    #[serde(default)]
    pub workflows: WorkflowsSettings,
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
    /// Legacy config field (ignored). JSON sidecar calls always use OpenAI `json_object` mode.
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

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            base_url: "http://localhost:11434/v1".into(),
            model: "unknown".into(),
            context_limit: 64_000,
            log_page_lines: default_log_page_lines(),
            max_log_pages: default_max_log_pages(),
            concurrency: default_llm_concurrency(),
            structured_output: default_structured_output(),
            max_output_tokens: default_max_output_tokens(),
            think: default_llm_think(),
            max_thinking_tokens: default_max_thinking_tokens(),
            reasoning_summary_tokens: default_reasoning_summary_tokens(),
            history_summary_tokens: default_history_summary_tokens(),
            api_key: None,
        }
    }
}

/// YAML `llm` is either a profile map (`ollama-qwen: {…}`) or a legacy single endpoint.
fn deserialize_llm_profiles<'de, D>(
    deserializer: D,
) -> std::result::Result<HashMap<String, LlmConfig>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<Value>::deserialize(deserializer)?;
    let Some(value) = value else {
        return Ok(HashMap::new());
    };
    let Value::Mapping(map) = value else {
        return Err(D::Error::custom("llm must be a YAML mapping"));
    };
    if map.contains_key(Value::String("base_url".into())) {
        let cfg: LlmConfig = serde_yaml::from_value(Value::Mapping(map))
            .map_err(|e| D::Error::custom(format!("invalid llm endpoint: {e}")))?;
        let mut profiles = HashMap::new();
        profiles.insert("default".into(), cfg);
        return Ok(profiles);
    }
    let mut profiles = HashMap::new();
    for (key, val) in map {
        let name = key.as_str().ok_or_else(|| {
            D::Error::custom("llm profile names must be strings (e.g. ollama-qwen)")
        })?;
        let cfg: LlmConfig = serde_yaml::from_value(val)
            .map_err(|e| D::Error::custom(format!("invalid llm profile `{name}`: {e}")))?;
        profiles.insert(name.to_string(), cfg);
    }
    Ok(profiles)
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
    /// Technique skill names to auto-load when this server's tools are warmed in chat.
    #[serde(default)]
    pub skills: Vec<String>,
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

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct WorkflowsSettings {
    /// When true, batch workflows may call readonly third-party MCP tools (default: false).
    #[serde(default)]
    pub mcp_readonly: bool,
    #[serde(flatten)]
    entries: HashMap<String, WorkflowConfig>,
}

impl WorkflowsSettings {
    pub fn get(&self, id: &str) -> Option<&WorkflowConfig> {
        self.entries.get(id)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &WorkflowConfig)> {
        self.entries.iter()
    }

    /// Effective readonly-MCP flag for a workflow (per-workflow override, else global).
    pub fn mcp_readonly_for(&self, workflow_id: &str) -> bool {
        self.entries
            .get(workflow_id)
            .and_then(|w| w.mcp_readonly)
            .unwrap_or(self.mcp_readonly)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WorkflowConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Technique skills; default from built-in workflow registry.
    #[serde(default)]
    pub skills: Vec<String>,
    pub schedule: Option<String>,
    /// Override `workflows.mcp_readonly` for this workflow.
    #[serde(default)]
    pub mcp_readonly: Option<bool>,
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

/// Compaction strategy name (code/ops/generic).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatCompactionStrategy {
    /// Preserve paths, errors, edit targets (coding chat default).
    #[default]
    Code,
    /// Preserve CI_KIND, verdicts, PR refs, digest excerpts (ops / MCP triage).
    Ops,
    /// Generic LLM rolling summary without domain-specific keep rules.
    Generic,
}

/// Session history / tool-result compaction for chat.
///
/// Accepts either a plain string (`chat.compaction: code`) or an object
/// (`chat.compaction: { strategy: code, summary_model: fast }`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ChatCompaction {
    #[serde(default)]
    pub strategy: ChatCompactionStrategy,
    /// Optional LLM profile name for compaction summaries. When set, uses a
    /// lighter/faster model for context compression instead of the main chat LLM.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary_model: Option<String>,
}

impl<'de> Deserialize<'de> for ChatCompaction {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        // Use serde_yaml::Value as intermediary to handle both string and mapping.
        #[derive(Deserialize)]
        struct Helper {
            #[serde(default)]
            strategy: ChatCompactionStrategy,
            #[serde(default)]
            summary_model: Option<String>,
        }
        let value = serde_yaml::Value::deserialize(deserializer)?;
        match value {
            Value::String(s) => {
                let strategy = match s.as_str() {
                    "code" => ChatCompactionStrategy::Code,
                    "ops" => ChatCompactionStrategy::Ops,
                    "generic" => ChatCompactionStrategy::Generic,
                    other => {
                        return Err(DeError::unknown_variant(other, &["code", "ops", "generic"]))
                    }
                };
                Ok(ChatCompaction {
                    strategy,
                    summary_model: None,
                })
            }
            Value::Mapping(_) => {
                let helper: Helper = serde_yaml::from_value(value)
                    .map_err(|e| DeError::custom(format!("invalid compaction config: {e}")))?;
                Ok(ChatCompaction {
                    strategy: helper.strategy,
                    summary_model: helper.summary_model,
                })
            }
            _ => Err(DeError::custom(
                "expected a string or mapping for `compaction`",
            )),
        }
    }
}

impl ChatCompaction {
    pub fn to_strategy(&self) -> crate::agent::context::CompactionStrategy {
        match self.strategy {
            ChatCompactionStrategy::Code => crate::agent::context::CompactionStrategy::Code,
            ChatCompactionStrategy::Ops => crate::agent::context::CompactionStrategy::Ops,
            ChatCompactionStrategy::Generic => crate::agent::context::CompactionStrategy::Generic,
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
    /// When the model streams reasoning but no assistant text/tool_calls, stop the
    /// stream after this many seconds (avoids waiting the full ~90s stream wall). 0 = off.
    #[serde(default = "default_chat_reasoning_only_warn_secs")]
    pub reasoning_only_warn_secs: u64,
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

fn default_chat_reasoning_only_warn_secs() -> u64 {
    30
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
            reasoning_only_warn_secs: default_chat_reasoning_only_warn_secs(),
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
    /// When set, `/api/*` and `/ws` require `Authorization: Bearer <token>`.
    #[serde(default)]
    pub auth_token: Option<String>,
}

fn default_web_bind() -> String {
    "127.0.0.1:8787".into()
}

impl Default for WebConfig {
    fn default() -> Self {
        Self {
            bind: default_web_bind(),
            auth_token: None,
        }
    }
}

impl WebConfig {
    /// Non-empty `auth_token` after trim — enables API/WebSocket bearer auth.
    pub fn auth_enabled(&self) -> bool {
        self.auth_token
            .as_deref()
            .is_some_and(|t| !t.trim().is_empty())
    }

    /// Trimmed bearer secret when auth is enabled.
    pub fn effective_auth_token(&self) -> Option<&str> {
        self.auth_token
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
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
        let path = path.as_ref();
        let raw = std::fs::read_to_string(path)?;
        let mut cfg: Config = match serde_yaml::from_str(&raw) {
            Ok(c) => c,
            Err(e) => {
                let loc = e
                    .location()
                    .map(|p| format!(" at line {}, column {}", p.line(), p.column()))
                    .unwrap_or_default();
                return Err(CoworkerError::Config(format!(
                    "invalid YAML in {}:{}{}",
                    path.display(),
                    loc,
                    e
                )));
            }
        };
        cfg.resolve_env_in_github();
        cfg.resolve_env_in_mcp();
        if let Some(name) = Config::read_llm_profile_sidecar(path) {
            cfg.llm_profile = Some(name);
        }
        cfg.finalize()?;
        Ok(cfg)
    }

    pub fn load_from_str(raw: &str) -> Result<Self> {
        let mut cfg: Config = match serde_yaml::from_str(raw) {
            Ok(c) => c,
            Err(e) => {
                let loc = e
                    .location()
                    .map(|p| format!(" at line {}, column {}", p.line(), p.column()))
                    .unwrap_or_default();
                return Err(CoworkerError::Config(format!("invalid YAML{loc}{e}")));
            }
        };
        cfg.resolve_env_in_github();
        cfg.resolve_env_in_mcp();
        cfg.finalize()?;
        Ok(cfg)
    }

    /// Sidecar next to `coworker.yaml` — stores last-selected profile without rewriting YAML comments.
    pub fn llm_profile_sidecar_path(config_path: &Path) -> PathBuf {
        config_path.with_extension("llm-profile")
    }

    pub fn read_llm_profile_sidecar(config_path: &Path) -> Option<String> {
        let path = Self::llm_profile_sidecar_path(config_path);
        let raw = std::fs::read_to_string(path).ok()?;
        let name = raw.trim();
        if name.is_empty() {
            None
        } else {
            Some(name.to_string())
        }
    }

    pub fn write_llm_profile_sidecar(config_path: &Path, profile: &str) -> Result<()> {
        let path = Self::llm_profile_sidecar_path(config_path);
        std::fs::write(path, format!("{profile}\n"))?;
        Ok(())
    }

    /// Sorted profile names for UI pickers.
    pub fn llm_profile_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.llm_profiles.keys().cloned().collect();
        names.sort();
        names
    }

    pub fn apply_llm_profile(&mut self) {
        if self.llm_profiles.is_empty() {
            tracing::warn!("coworker.yaml has no `llm` profiles configured");
            return;
        }
        let name = self
            .llm_profile
            .clone()
            .filter(|n| self.llm_profiles.contains_key(n))
            .or_else(|| self.llm_profile_names().into_iter().next());
        let Some(name) = name else {
            return;
        };
        let Some(profile) = self.llm_profiles.get(&name).cloned() else {
            return;
        };
        self.llm_profile = Some(name);
        self.llm = profile;
    }

    pub fn switch_llm_profile(&mut self, name: &str) -> Result<()> {
        if !self.llm_profiles.contains_key(name) {
            return Err(CoworkerError::Config(format!(
                "unknown llm profile `{name}` (available: {})",
                self.llm_profile_names().join(", ")
            )));
        }
        self.llm_profile = Some(name.to_string());
        self.apply_llm_profile();
        Ok(())
    }

    /// Reserved for derived fields after YAML deserialization.
    pub fn finalize(&mut self) -> Result<()> {
        self.chat.workspace = expand_tilde(&self.chat.workspace.to_string_lossy());
        if let Ok(canonical) = self.chat.workspace.canonicalize() {
            self.chat.workspace = canonical;
        }
        if self.theme.is_some() && self.tui.theme.is_some() {
            tracing::warn!(
                "coworker.yaml sets both `theme` and `tui.theme` — using top-level `theme`"
            );
        }
        if self.llm_profiles.is_empty() {
            return Err(CoworkerError::Config(
                "coworker.yaml must define at least one LLM under `llm`".into(),
            ));
        }
        self.apply_llm_profile();
        Ok(())
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
            "coworker.yaml not found (cwd or .coworker/) — run 'unistar-coworker init' to create one".into(),
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
        let cfg = Config::load_from_str(yaml).unwrap();
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

    #[test]
    fn mcp_server_skills_parse() {
        let yaml = r#"
llm: { base_url: http://localhost:11434/v1, model: m, context_limit: 64000 }
repos: [org/repo]
mcp:
  servers:
    - id: slack
      transport: stdio
      command: npx
      skills: [slack-ops, github-ops-tone]
"#;
        let cfg = Config::load_from_str(yaml).expect("parse");
        assert_eq!(
            cfg.mcp.servers[0].skills,
            vec!["slack-ops", "github-ops-tone"]
        );
    }

    #[test]
    fn web_auth_token_deserializes() {
        let yaml = r#"
llm: { base_url: http://localhost:11434/v1, model: m, context_limit: 64000 }
repos: [acme/widget]
web:
  bind: 0.0.0.0:8787
  auth_token: secret
"#;
        let cfg: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(cfg.web.bind, "0.0.0.0:8787");
        assert_eq!(cfg.web.auth_token.as_deref(), Some("secret"));
        assert!(cfg.web.auth_enabled());
        assert_eq!(cfg.web.effective_auth_token(), Some("secret"));
    }

    #[test]
    fn web_auth_token_defaults_none() {
        let yaml = r#"
llm: { base_url: http://localhost:11434/v1, model: m, context_limit: 64000 }
repos: [acme/widget]
"#;
        let cfg: Config = serde_yaml::from_str(yaml).unwrap();
        assert!(cfg.web.auth_token.is_none());
        assert!(!cfg.web.auth_enabled());
    }

    #[test]
    fn llm_profile_resolves_active_config() {
        let yaml = r#"
llm_profile: fast
llm:
  fast:
    base_url: http://127.0.0.1:11434/v1
    model: qwen-fast
    context_limit: 32000
  slow:
    base_url: http://127.0.0.1:11434/v1
    model: qwen-slow
    context_limit: 128000
repos: [acme/widget]
"#;
        let mut cfg = Config::load_from_str(yaml).unwrap();
        assert_eq!(cfg.llm.model, "qwen-fast");
        assert_eq!(cfg.llm_profile.as_deref(), Some("fast"));
        cfg.switch_llm_profile("slow").unwrap();
        assert_eq!(cfg.llm.model, "qwen-slow");
    }

    #[test]
    fn legacy_single_llm_block_becomes_default_profile() {
        let yaml = r#"
llm:
  base_url: http://localhost:11434/v1
  model: gemma
  context_limit: 64000
repos: [acme/widget]
"#;
        let cfg = Config::load_from_str(yaml).unwrap();
        assert_eq!(cfg.llm.model, "gemma");
        assert_eq!(cfg.llm_profile.as_deref(), Some("default"));
        assert!(cfg.llm_profiles.contains_key("default"));
    }
}
