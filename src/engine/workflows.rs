use std::path::PathBuf;

use crate::agent::harness_errors::{generic_tool_failure_envelope, workflow_error};
use crate::config::{Config, WorkflowConfig};
use crate::error::{CoworkerError, Result};

tokio::task_local! {
    /// When set, workflow execution is in flight; value = readonly MCP allowed.
    static WORKFLOW_MCP_READONLY: bool;
}

/// Run `f` with workflow MCP policy (readonly third-party MCP on/off).
pub async fn workflow_mcp_scope<T>(mcp_readonly: bool, f: impl std::future::Future<Output = T>) -> T {
    WORKFLOW_MCP_READONLY.scope(mcp_readonly, f).await
}

/// Guard third-party MCP during batch workflows (chat path leaves task-local unset).
pub fn check_workflow_mcp_allowed(tool_name: &str, mutating: bool) -> Result<()> {
    match WORKFLOW_MCP_READONLY.try_with(|&allowed| allowed) {
        Err(_) => Ok(()),
        Ok(false) => Err(workflow_mcp_blocked_error(tool_name)),
        Ok(true) if mutating => Err(workflow_mcp_mutating_blocked_error(tool_name)),
        Ok(true) => Ok(()),
    }
}

fn workflow_mcp_blocked_error(tool_name: &str) -> CoworkerError {
    workflow_error(
        generic_tool_failure_envelope(
            tool_name,
            "Third-party MCP is disabled for batch workflows",
            "Workflows use GithubHarness by default; federated MCP is chat-only unless enabled in coworker.yaml",
            vec![
                "Set workflows.<id>.mcp_readonly: true to allow readonly MCP for one workflow".into(),
                "Or set workflows.mcp_readonly: true as a global default".into(),
                "Use chat for ad-hoc MCP tool chains".into(),
            ],
            None,
            "",
        )
        .with_code("WORKFLOW_MCP_BLOCKED"),
    )
}

fn workflow_mcp_mutating_blocked_error(tool_name: &str) -> CoworkerError {
    workflow_error(
        generic_tool_failure_envelope(
            tool_name,
            "Mutating MCP tools are not allowed in batch workflows",
            "Workflow MCP whitelist permits readonly tools only; mutating tools require chat + approval",
            vec![
                "Use chat with human approval for mutating MCP tools".into(),
                "Prefer GithubHarness tools for CI/PR mutations in workflows".into(),
            ],
            None,
            "",
        )
        .with_code("WORKFLOW_MCP_MUTATING_BLOCKED"),
    )
}

#[derive(Debug, Clone)]
pub struct WorkflowDef {
    pub id: String,
    pub skill_paths: Vec<PathBuf>,
    pub schedule: Option<String>,
    pub mcp_readonly: bool,
}

impl WorkflowDef {
    pub fn skill_paths(&self) -> &[PathBuf] {
        &self.skill_paths
    }
}

pub struct WorkflowRunner<'a> {
    config: &'a Config,
}

impl<'a> WorkflowRunner<'a> {
    pub fn new(config: &'a Config) -> Self {
        Self { config }
    }

    pub fn get(&self, id: &str) -> Result<WorkflowDef> {
        let w = self
            .config
            .workflows
            .get(id)
            .ok_or_else(|| CoworkerError::Workflow(format!("unknown workflow: {id}")))?;
        if !w.enabled {
            return Err(CoworkerError::Workflow(format!(
                "workflow {id} is disabled"
            )));
        }
        Ok(self.to_def(id, w))
    }

    pub fn mcp_readonly_enabled(&self, workflow_id: &str) -> bool {
        self.config.workflows.mcp_readonly_for(workflow_id)
    }

    fn to_def(&self, id: &str, w: &WorkflowConfig) -> WorkflowDef {
        let skill_paths = resolve_skill_paths(id, w);
        WorkflowDef {
            id: id.to_string(),
            skill_paths,
            schedule: w.schedule.clone(),
            mcp_readonly: self.config.workflows.mcp_readonly_for(id),
        }
    }
}

fn resolve_skill_paths(id: &str, w: &WorkflowConfig) -> Vec<PathBuf> {
    if !w.skills.is_empty() {
        return w.skills.iter().map(PathBuf::from).collect();
    }
    super::workflow_registry::default_skill_paths(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn mcp_readonly_defaults_false() {
        let yaml = r#"
llm: { base_url: http://localhost:11434/v1, model: m, context_limit: 64000 }
repos: [org/repo]
workflows:
  daily-work: {}
"#;
        let cfg = Config::load_from_str(yaml).unwrap();
        assert!(!cfg.workflows.mcp_readonly);
        assert!(!cfg.workflows.mcp_readonly_for("daily-work"));
    }

    #[test]
    fn per_workflow_mcp_readonly_override() {
        let yaml = r#"
llm: { base_url: http://localhost:11434/v1, model: m, context_limit: 64000 }
repos: [org/repo]
workflows:
  mcp_readonly: false
  daily-work:
    mcp_readonly: true
  review-radar: {}
"#;
        let cfg = Config::load_from_str(yaml).unwrap();
        assert!(cfg.workflows.mcp_readonly_for("daily-work"));
        assert!(!cfg.workflows.mcp_readonly_for("review-radar"));
    }

    #[tokio::test]
    async fn workflow_scope_blocks_mcp_when_disabled() {
        let err = workflow_mcp_scope(false, async {
            check_workflow_mcp_allowed("slack_post_message", false)
        })
        .await
        .expect_err("expected block");
        assert!(err.to_string().contains("WORKFLOW_MCP_BLOCKED"));
    }

    #[tokio::test]
    async fn workflow_scope_allows_readonly_mcp_when_enabled() {
        workflow_mcp_scope(true, async {
            check_workflow_mcp_allowed("slack_list_channels", false).unwrap();
        })
        .await;
    }

    #[tokio::test]
    async fn workflow_scope_blocks_mutating_mcp_even_when_enabled() {
        let err = workflow_mcp_scope(true, async {
            check_workflow_mcp_allowed("slack_post_message", true)
        })
        .await
        .expect_err("expected mutating block");
        assert!(err.to_string().contains("WORKFLOW_MCP_MUTATING_BLOCKED"));
    }

    #[tokio::test]
    async fn outside_workflow_scope_allows_mcp_checks() {
        check_workflow_mcp_allowed("slack_post_message", true).unwrap();
    }
}
