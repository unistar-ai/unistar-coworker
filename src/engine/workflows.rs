use std::path::PathBuf;

use crate::config::{Config, WorkflowConfig};
use crate::error::{CoworkerError, Result};

#[derive(Debug, Clone)]
pub struct WorkflowDef {
    pub id: String,
    pub skill_paths: Vec<PathBuf>,
    pub schedule: Option<String>,
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

    fn to_def(&self, id: &str, w: &WorkflowConfig) -> WorkflowDef {
        let skill_paths = resolve_skill_paths(id, w);
        WorkflowDef {
            id: id.to_string(),
            skill_paths,
            schedule: w.schedule.clone(),
        }
    }
}

fn resolve_skill_paths(id: &str, w: &WorkflowConfig) -> Vec<PathBuf> {
    if !w.skills.is_empty() {
        return w.skills.iter().map(PathBuf::from).collect();
    }
    super::workflow_registry::default_skill_paths(id)
}
