//! Built-in workflow catalog — metadata and default skills (Rust SSOT).

use std::path::PathBuf;

use crate::error::{CoworkerError, Result};

#[derive(Debug, Clone, Copy)]
pub struct WorkflowMeta {
    pub id: &'static str,
    pub description: &'static str,
    pub default_skills: &'static [&'static str],
}

/// Batch workflows only — PR triage (`harness_triage_pr`), store reports (`report *`),
/// and GitHub harness tools cover everything else.
pub const WORKFLOWS: &[WorkflowMeta] = &[
    WorkflowMeta {
        id: "daily-work",
        description: "Morning GitHub triage digest across configured repos.",
        default_skills: &["ci-triage", "digest-style"],
    },
    WorkflowMeta {
        id: "review-radar",
        description: "List PRs that are CI-green but blocked on review.",
        default_skills: &["pr-merge", "digest-style"],
    },
];

pub fn lookup(id: &str) -> Option<&'static WorkflowMeta> {
    WORKFLOWS.iter().find(|w| w.id == id)
}

pub fn require(id: &str) -> Result<&'static WorkflowMeta> {
    lookup(id).ok_or_else(|| CoworkerError::Workflow(format!("unknown workflow: {id}")))
}

pub fn default_skill_paths(id: &str) -> Vec<PathBuf> {
    lookup(id)
        .map(|m| {
            m.default_skills
                .iter()
                .map(|r| super::skill::resolve_skill_ref(r))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_covers_core_workflows() {
        assert!(lookup("daily-work").is_some());
        assert!(lookup("review-radar").is_some());
        assert!(lookup("issue-triage").is_none());
    }

    #[test]
    fn default_skills_resolve() {
        let paths = default_skill_paths("daily-work");
        assert_eq!(paths.len(), 2);
        assert!(paths[0].to_string_lossy().contains("ci-triage"));
    }
}
