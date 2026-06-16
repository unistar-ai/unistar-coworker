use std::path::{Path, PathBuf};

use crate::config::{Config, WorkflowConfig};
use crate::error::{CoworkerError, Result};

#[derive(Debug, Clone)]
pub struct WorkflowDef {
    pub id: String,
    pub agent: PathBuf,
    pub skill_paths: Vec<PathBuf>,
    pub schedule: Option<String>,
}

impl WorkflowDef {
    pub fn agent_path(&self) -> &PathBuf {
        &self.agent
    }

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

    fn to_def(&self, id: &str, w: &WorkflowConfig) -> WorkflowDef {
        let agent = resolve_agent_path(id, w);
        let skill_paths = resolve_skill_paths(id, w, &agent);
        WorkflowDef {
            id: id.to_string(),
            agent,
            skill_paths,
            schedule: w.schedule.clone(),
        }
    }
}

fn resolve_agent_path(id: &str, w: &WorkflowConfig) -> PathBuf {
    if let Some(agent) = &w.agent {
        return PathBuf::from(agent);
    }
    PathBuf::from(format!("agents/{id}/AGENT.md"))
}

fn resolve_skill_paths(id: &str, w: &WorkflowConfig, agent_path: &Path) -> Vec<PathBuf> {
    if !w.skills.is_empty() {
        return w.skills.iter().map(PathBuf::from).collect();
    }

    if let Ok(agent) = crate::engine::skill::load_agent(agent_path) {
        if !agent.skill_refs.is_empty() {
            return agent
                .skill_refs
                .iter()
                .map(|r| crate::engine::skill::resolve_skill_ref(r))
                .collect();
        }
    }

    crate::engine::prompt::default_workflow_skill_paths(id)
}
