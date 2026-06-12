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

#[derive(Debug, Clone, Deserialize, Serialize)]
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
    pub skill: String,
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
