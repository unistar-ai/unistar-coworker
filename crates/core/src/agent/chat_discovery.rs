//! Progressive tool discovery: session cache, intent hints, schema warmup.

use std::collections::HashSet;

use crate::agent::tool_catalog;
use crate::engine::{SkillRegistry, SkillSpec};

const TOOL_LIST_CACHE_NOTE: &str = "\n\n(session cache — avoid repeating tool_list)";

/// Per-chat-session discovery state (tool list cache + warmed native schemas).
#[derive(Debug)]
pub struct ChatDiscoveryState {
    pub tool_list_cache: Option<String>,
    pub warmed_tools: HashSet<String>,
    pub skill_registry: SkillRegistry,
    pub loaded_skills: HashSet<String>,
}

/// Legacy alias — file basics are in `PRELOAD_NATIVE_TOOLS`; nothing extra to warm at cold start.
pub const PINNED_WARM_TOOLS: &[&str] = &[];

#[cfg(test)]
const PR_TASK_WARM_TOOLS: &[&str] = &[
    "pr_get_overview",
    "pr_list_changed_files",
    "pr_get_diff",
    "pr_get_ci_snapshot",
];

impl Default for ChatDiscoveryState {
    fn default() -> Self {
        Self {
            tool_list_cache: None,
            warmed_tools: HashSet::new(),
            skill_registry: SkillRegistry { skills: Vec::new() },
            loaded_skills: HashSet::new(),
        }
    }
}

impl ChatDiscoveryState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Bootstrap chat discovery: registry + optional eager skills (native mode only).
    pub fn with_bootstrap(
        _user_message: &str,
        registry: SkillRegistry,
        initial_skills: &[SkillSpec],
    ) -> Self {
        let mut state = Self {
            skill_registry: registry,
            ..Self::new()
        };
        if initial_skills.is_empty() {
            return state;
        }
        for skill in initial_skills {
            state.loaded_skills.insert(skill.name.clone());
        }
        for tool in SkillRegistry::collect_tool_refs(initial_skills) {
            state.warm_tool(&tool);
        }
        for name in PINNED_WARM_TOOLS {
            state.warm_tool(name);
        }
        state
    }

    #[cfg(test)]
    /// Optional PR-task warmup (not used in lazy chat cold start).
    pub fn apply_github_pr_bootstrap(state: &mut Self, user_message: &str) {
        if !message_looks_like_pr_task(user_message) {
            return;
        }
        if let Some(skill) = state.skill_registry.get("pr-review").cloned() {
            state.warm_skill_tools(&skill);
        }
        let lower = user_message.to_ascii_lowercase();
        if lower.contains("ci") || lower.contains("failing") || lower.contains("check") {
            if let Some(skill) = state.skill_registry.get("ci-triage").cloned() {
                state.warm_skill_tools(&skill);
            }
        }
        for tool in PR_TASK_WARM_TOOLS {
            state.warm_tool(tool);
        }
    }

    pub fn warm_skill_tools(&mut self, skill: &SkillSpec) {
        self.loaded_skills.insert(skill.name.clone());
        for tool in &skill.tool_refs {
            self.warm_tool(tool);
        }
    }

    /// Restore warmed skills/tools from prior successful `skill_load` tool rows in the session.
    pub fn rehydrate_from_tool_history(&mut self, messages: &[crate::store::ChatMessage]) {
        use crate::store::ChatRole;
        for msg in messages {
            if msg.role != ChatRole::Tool || msg.tool_name.as_deref() != Some("skill_load") {
                continue;
            }
            if !msg.content.starts_with("tool_result(") {
                continue;
            }
            let Some(args_json) = msg.tool_calls_json.as_deref() else {
                continue;
            };
            let Ok(args) = serde_json::from_str::<serde_json::Value>(args_json) else {
                continue;
            };
            let Some(name) = args.get("name").and_then(|v| v.as_str()) else {
                continue;
            };
            if let Some(skill) = self.skill_registry.get(name).cloned() {
                self.warm_skill_tools(&skill);
            }
        }
    }

    pub fn cached_tool_list(&self) -> Option<String> {
        self.tool_list_cache
            .as_ref()
            .map(|text| format!("{text}{TOOL_LIST_CACHE_NOTE}"))
    }

    pub fn store_tool_list(&mut self, text: String) {
        if !text.trim().is_empty() {
            self.tool_list_cache = Some(text);
        }
    }

    pub fn warm_tool(&mut self, name: &str) {
        let name = name.trim();
        if name.is_empty() || tool_catalog::is_lazy_native_tool(name) {
            return;
        }
        if tool_catalog::is_catalog_tool(name) {
            self.warmed_tools.insert(name.to_string());
        }
    }

    /// Warm federated MCP tool names when present in the registry.
    pub async fn warm_tool_from_registry(&mut self, name: &str, mcp: &crate::mcp::McpPool) {
        self.warm_tool(name);
        let server_id = mcp.server_id_for_tool(name).await;
        let is_mcp = server_id.is_some() || mcp.is_mcp_tool_async(name).await;
        if !self.warmed_tools.contains(name) && is_mcp {
            self.warmed_tools.insert(name.to_string());
        }
        if let Some(server_id) = server_id {
            self.load_configured_server_skills(mcp, &server_id);
        }
    }

    /// Load `mcp.servers[].skills` into session discovery when an MCP server is warmed.
    fn load_configured_server_skills(&mut self, mcp: &crate::mcp::McpPool, server_id: &str) {
        for skill_name in mcp.server_skills(server_id) {
            if let Some(skill) = self.skill_registry.get(&skill_name).cloned() {
                self.warm_skill_tools(&skill);
            }
        }
    }

    pub fn warm_from_tool_call_args(&mut self, tool_name: &str, args: &serde_json::Value) {
        if tool_name == "tool_call" {
            if let Some(inner) = args.get("name").and_then(|v| v.as_str()) {
                self.warm_tool(inner);
            }
        } else if !tool_catalog::is_lazy_native_tool(tool_name) {
            self.warm_tool(tool_name);
        }
    }

    /// Loaded technique skills (injected under `## Techniques` in the system prompt).
    pub fn loaded_skill_specs(&self) -> Vec<SkillSpec> {
        self.skill_registry
            .skills
            .iter()
            .filter(|s| self.loaded_skills.contains(&s.name))
            .cloned()
            .collect()
    }
}

/// Pull request number from user text (`PR #42`, `#19264`, GitHub pull URLs, etc.).
pub fn extract_pr_number_for_autofill(lower: &str) -> Option<u32> {
    if let Some(idx) = lower.find("/pull/") {
        let rest = &lower[idx + 6..];
        if let Some(n) = parse_u32_prefix(rest) {
            return Some(n);
        }
    }
    for (i, ch) in lower.char_indices() {
        if ch == '#' {
            if let Some(n) = parse_u32_prefix(&lower[i + 1..]) {
                return Some(n);
            }
        }
        if lower[i..].starts_with("pr") {
            let rest = &lower[i + 2..];
            let rest = rest.trim_start();
            if let Some(n) = parse_u32_prefix(rest) {
                return Some(n);
            }
        }
    }
    None
}

/// Parse `github.com/org/repo/pull/N` (or issues) into `(org/repo, N)`.
pub fn extract_github_pr_link(text: &str) -> Option<(String, u32)> {
    let lower = text.to_ascii_lowercase();
    if !lower.contains("github.com/") {
        return None;
    }
    let pr = extract_pr_number_for_autofill(&lower)?;
    let repo = extract_github_repo_slug(text)?;
    Some((repo, pr))
}

/// `owner/repo` from a GitHub URL or `github.com/owner/repo/...` fragment.
pub fn extract_github_repo_slug(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    let marker = "github.com/";
    let idx = lower.find(marker)?;
    let rest = &text[idx + marker.len()..];
    let owner = rest.split(&['/', '?', '#'][..]).next()?.trim();
    let after_owner = &rest[owner.len()..];
    let name = after_owner
        .trim_start_matches('/')
        .split(&['/', '?', '#'][..])
        .next()?
        .trim()
        .trim_end_matches(".git");
    if owner.is_empty() || name.is_empty() {
        return None;
    }
    Some(format!("{owner}/{name}"))
}

/// User message is about analyzing/reviewing a specific PR (not generic git).
pub fn message_looks_like_pr_task(text: &str) -> bool {
    if extract_github_pr_link(text).is_some() {
        return true;
    }
    let lower = text.to_ascii_lowercase();
    let prish = lower.contains("/pull/")
        || lower.contains("pull request")
        || lower.contains("merge request")
        || extract_pr_number_for_autofill(&lower).is_some()
            && (lower.contains("pr") || lower.contains("pull"));
    if !prish {
        return false;
    }
    lower.contains("analyze")
        || lower.contains("review")
        || lower.contains("分析")
        || lower.contains("审查")
        || lower.contains("inspect")
        || lower.contains("look at")
        || lower.contains("看看")
        || lower.contains("diff")
        || lower.contains("change")
}

fn parse_u32_prefix(s: &str) -> Option<u32> {
    let digits: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
    if !digits.is_empty() {
        digits.parse().ok()
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::SkillSpec;

    #[test]
    fn tool_list_cache_note() {
        let mut state = ChatDiscoveryState::new();
        state.store_tool_list("2 tool(s)".into());
        assert!(state.cached_tool_list().unwrap().contains("session cache"));
    }

    #[test]
    fn warm_skips_meta_tools() {
        let mut state = ChatDiscoveryState::new();
        state.warm_tool("tool_search");
        assert!(state.warmed_tools.is_empty());
        state.warm_tool("pr_get_overview");
        assert!(state.warmed_tools.contains("pr_get_overview"));
    }

    #[test]
    fn bootstrap_warms_skill_tools() {
        use crate::engine::SkillRegistry;
        let registry = SkillRegistry::from_skills(vec![SkillSpec {
            name: "ci-triage".into(),
            description: String::new(),
            body: String::new(),
            skill_refs: vec![],
            tool_refs: vec!["ci_get_failure_digest".into()],
            always_load: false,
            ..Default::default()
        }]);
        let skills = vec![SkillSpec {
            name: "ci-triage".into(),
            description: String::new(),
            body: String::new(),
            skill_refs: vec![],
            tool_refs: vec!["ci_get_failure_digest".into()],
            always_load: false,
            ..Default::default()
        }];
        let state =
            ChatDiscoveryState::with_bootstrap("Why is CI failing on PR #42?", registry, &skills);
        assert!(state.warmed_tools.contains("ci_get_failure_digest"));
        assert!(!state.warmed_tools.contains("read_file"));
    }

    #[test]
    fn bootstrap_lazy_cold_start_warms_nothing() {
        use crate::engine::SkillRegistry;
        let state =
            ChatDiscoveryState::with_bootstrap("hello", SkillRegistry::from_skills(vec![]), &[]);
        assert!(state.warmed_tools.is_empty());
        assert!(state.loaded_skills.is_empty());
    }

    #[test]
    fn loaded_skill_specs_follows_registry() {
        use crate::engine::SkillRegistry;
        let registry = SkillRegistry::from_skills(vec![SkillSpec {
            name: "ci-triage".into(),
            description: String::new(),
            body: String::new(),
            skill_refs: vec![],
            tool_refs: vec![],
            always_load: false,
            ..Default::default()
        }]);
        let mut state = ChatDiscoveryState::with_bootstrap("ci fail", registry, &[]);
        state.warm_skill_tools(&SkillSpec {
            name: "ci-triage".into(),
            description: String::new(),
            body: String::new(),
            skill_refs: vec![],
            tool_refs: vec![],
            always_load: false,
            ..Default::default()
        });
        assert_eq!(state.loaded_skill_specs().len(), 1);
        assert_eq!(state.loaded_skill_specs()[0].name, "ci-triage");
    }

    #[test]
    fn extract_pr_from_hash_and_pr_prefix() {
        assert_eq!(
            extract_pr_number_for_autofill("why is ci failing on pr #19264?"),
            Some(19264)
        );
        assert_eq!(
            extract_pr_number_for_autofill("check https://github.com/o/r/pull/99"),
            Some(99)
        );
    }

    #[test]
    fn extract_github_pr_link_parses_url() {
        let url = "分析 https://github.com/acme/widget/pull/42";
        assert_eq!(
            extract_github_pr_link(url),
            Some(("acme/widget".into(), 42))
        );
    }

    #[test]
    fn message_looks_like_pr_task_chinese() {
        assert!(message_looks_like_pr_task(
            "分析这个 PR: https://github.com/acme/widget/pull/42"
        ));
        assert!(!message_looks_like_pr_task(
            "fix the login bug in src/auth.rs"
        ));
    }

    #[test]
    fn pr_bootstrap_warms_harness_tools() {
        use crate::engine::SkillRegistry;
        let registry = SkillRegistry::from_skills(vec![SkillSpec {
            name: "pr-review".into(),
            description: String::new(),
            body: String::new(),
            skill_refs: vec![],
            tool_refs: vec!["pr_get_overview".into(), "pr_get_diff".into()],
            always_load: false,
            ..Default::default()
        }]);
        let mut state = ChatDiscoveryState::with_bootstrap(
            "analyze https://github.com/o/r/pull/12",
            registry,
            &[],
        );
        ChatDiscoveryState::apply_github_pr_bootstrap(
            &mut state,
            "analyze https://github.com/o/r/pull/12",
        );
        assert!(state.warmed_tools.contains("pr_get_overview"));
        assert!(state.warmed_tools.contains("pr_get_diff"));
        assert!(state.loaded_skills.contains("pr-review"));
    }

    #[test]
    fn rehydrate_from_tool_history_warms_prior_skill_load() {
        use crate::engine::SkillRegistry;
        use crate::store::{ChatMessage, ChatRole};
        use chrono::Utc;
        use uuid::Uuid;

        let registry = SkillRegistry::from_skills(vec![SkillSpec {
            name: "pr-review".into(),
            description: String::new(),
            body: String::new(),
            skill_refs: vec![],
            tool_refs: vec!["pr_get_overview".into()],
            always_load: false,
            ..Default::default()
        }]);
        let mut state = ChatDiscoveryState::with_bootstrap("review pr", registry, &[]);
        assert!(!state.loaded_skills.contains("pr-review"));

        let history = vec![ChatMessage {
            id: Uuid::new_v4(),
            session_id: Uuid::new_v4(),
            role: ChatRole::Tool,
            content: "tool_result(skill_load):\nargs: {\"name\":\"pr-review\"}\n\n### pr-review"
                .into(),
            ts: Utc::now(),
            tool_name: Some("skill_load".into()),
            tool_calls_json: Some(r#"{"name":"pr-review"}"#.into()),
            tool_call_id: None,
            reasoning_original: None,
            parent_message_id: None,
            branch_index: None,
        }];
        state.rehydrate_from_tool_history(&history);
        assert!(state.loaded_skills.contains("pr-review"));
        assert!(state.warmed_tools.contains("pr_get_overview"));
    }

    #[tokio::test]
    async fn warm_mcp_tool_loads_configured_server_skills() {
        use crate::config::{McpConfig, McpExposeConfig, McpServerConfig, McpTransport};
        use crate::engine::SkillRegistry;
        use crate::mcp::McpPool;
        use std::collections::HashMap;

        let registry = SkillRegistry::from_skills(vec![SkillSpec {
            name: "slack-ops".into(),
            description: String::new(),
            body: String::new(),
            skill_refs: vec![],
            tool_refs: vec!["slack_post_message".into()],
            always_load: false,
            ..Default::default()
        }]);
        let mut state = ChatDiscoveryState::with_bootstrap("post to slack", registry, &[]);
        let pool = McpPool::new(McpConfig {
            defaults: Default::default(),
            servers: vec![McpServerConfig {
                id: "slack".into(),
                enabled: true,
                transport: McpTransport::Stdio,
                command: None,
                args: vec![],
                env: HashMap::new(),
                url: None,
                headers: HashMap::new(),
                expose: McpExposeConfig {
                    prefix: Some("slack_".into()),
                    allowlist: vec![],
                    denylist: vec![],
                },
                approval: Default::default(),
                startup: None,
                timeout_secs: None,
                skills: vec!["slack-ops".into()],
            }],
        });
        state
            .warm_tool_from_registry("slack_post_message", &pool)
            .await;
        assert!(state.loaded_skills.contains("slack-ops"));
        assert!(state.warmed_tools.contains("slack_post_message"));
    }
}
