use std::path::PathBuf;

use crate::config::{Config, WorkflowConfig};
use crate::error::{CoworkerError, Result};

#[derive(Debug, Clone)]
pub struct WorkflowDef {
    pub id: String,
    pub enabled: bool,
    pub skill: PathBuf,
    pub schedule: Option<String>,
}

impl WorkflowDef {
    pub fn skill_path(&self) -> PathBuf {
        self.skill.clone()
    }
}

pub struct WorkflowRunner<'a> {
    config: &'a Config,
}

impl<'a> WorkflowRunner<'a> {
    pub fn new(config: &'a Config) -> Self {
        Self { config }
    }

    pub fn enabled(&self) -> Vec<WorkflowDef> {
        self.config
            .workflows
            .iter()
            .filter(|(_, w)| w.enabled)
            .map(|(id, w)| self.to_def(id, w))
            .collect()
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
        WorkflowDef {
            id: id.to_string(),
            enabled: w.enabled,
            skill: PathBuf::from(&w.skill),
            schedule: w.schedule.clone(),
        }
    }
}
