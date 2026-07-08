pub mod args;
pub mod backport;
pub mod checks;
pub mod ci;
pub mod ci_check_url;
pub mod ci_common;
pub mod ci_digest;
pub mod ci_fingerprint;
pub mod ci_health;
pub mod ci_logs;
pub mod ci_tier2;
pub mod ci_workflow_stats;
pub mod discovery;
pub mod error;
pub mod events;
pub mod exec;
pub mod harness;
pub mod helpers;
pub mod issue;
pub mod notify;
pub mod policy;
pub mod pr;
pub mod pr_batch;
pub mod pr_ci;
pub mod release;
pub mod repo;
pub mod resources;
pub mod security;

pub use harness::{spawn_github, GithubHarness};

use crate::config::ChatToolMode;

/// Lazy discovery is always native when GitHub harness is available.
pub fn effective_chat_tool_mode(configured: ChatToolMode, harness: &GithubHarness) -> ChatToolMode {
    match configured {
        ChatToolMode::Native => ChatToolMode::Native,
        ChatToolMode::Auto | ChatToolMode::Lazy if harness.supports_lazy_meta() => {
            ChatToolMode::Auto
        }
        ChatToolMode::Auto | ChatToolMode::Lazy => ChatToolMode::Native,
    }
}
