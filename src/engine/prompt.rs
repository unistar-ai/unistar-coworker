use std::path::PathBuf;

use crate::error::Result;

use super::skill::{load_skills, load_skills_from_refs, read_base_tools, AgentSpec, SkillSpec};

/// Classify output contract — harness field limits (not domain technique).
const CLASSIFY_OUTPUT_CONTRACT: &str = "\
You triage CI failures for a daily digest. Classify each failure and explain it clearly.\n\
\n\
Always fill ALL fields with specific, actionable content from the logs. Keep responses concise:\n\
- reason: one line, ≤120 characters\n\
- diagnosis: max 2 sentences, ≤320 characters — what failed, log evidence, merge impact\n\
- recommended_action: one sentence, ≤160 characters — concrete next step\n\
- test_name: failing test if identifiable\n\
\n\
You may receive one page of logs at a time; prior pages are summarized, not repeated. \
If inconclusive on this page, use verdict unknown and fill page_summary for the next page.";

#[derive(Debug, Clone)]
pub struct PromptBundle {
    pub agent: AgentSpec,
    pub skills: Vec<SkillSpec>,
    pub tools_doc: String,
    pub runtime_context: String,
}

#[derive(Debug, Clone)]
pub struct WorkflowSpec {
    pub agent: AgentSpec,
    pub skills: Vec<SkillSpec>,
}

pub fn load_tools_doc() -> Result<String> {
    let base = read_base_tools();
    if base.trim().is_empty() {
        tracing::warn!("missing skills/_base/TOOLS.md, using built-in tool summary");
        Ok(DEFAULT_TOOLS_DOC.into())
    } else {
        Ok(base)
    }
}

pub fn load_tools_doc_with_preferred(preferred: &[String]) -> Result<String> {
    let base = load_tools_doc()?;
    if preferred.is_empty() {
        Ok(base)
    } else {
        Ok(format!(
            "{base}\n\n## Preferred tools (this session)\n\
             Call tools via the API when you need data; reply in plain text when done.\n\
             {}",
            preferred
                .iter()
                .map(|t| format!("- `{t}`"))
                .collect::<Vec<_>>()
                .join("\n")
        ))
    }
}

const DEFAULT_TOOLS_DOC: &str = "See unistar-mcp lazy tools: tool_list, tool_describe, tool_call.";

pub fn join_skills(skills: &[SkillSpec]) -> String {
    if skills.is_empty() {
        return String::new();
    }
    skills
        .iter()
        .map(|s| {
            let title = if s.name.is_empty() {
                "technique".into()
            } else {
                s.name.clone()
            };
            format!("### {title}\n{}", s.body)
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub fn compose_system_prompt(bundle: &PromptBundle) -> String {
    let techniques = join_skills(&bundle.skills);
    if techniques.is_empty() {
        format!(
            "{}\n\n## Tools\n{}\n\n## Context\n{}",
            bundle.agent.body, bundle.tools_doc, bundle.runtime_context
        )
    } else {
        format!(
            "{}\n\n## Techniques\n{}\n\n## Tools\n{}\n\n## Context\n{}",
            bundle.agent.body, techniques, bundle.tools_doc, bundle.runtime_context
        )
    }
}

pub fn compose_classify_prompt(
    playbook_prefix: &str,
    skills: &[SkillSpec],
    task_agent: Option<&AgentSpec>,
) -> String {
    let mut parts = Vec::new();
    if !playbook_prefix.is_empty() {
        parts.push(playbook_prefix.trim().to_string());
    }
    if let Some(agent) = task_agent {
        if !agent.body.is_empty() {
            parts.push(agent.body.clone());
        }
    }
    let techniques = join_skills(skills);
    if !techniques.is_empty() {
        parts.push(techniques);
    }
    parts.push(CLASSIFY_OUTPUT_CONTRACT.to_string());
    parts.join("\n\n")
}

pub fn default_chat_agent_path() -> PathBuf {
    PathBuf::from("agents/chat/AGENT.md")
}

pub fn default_chat_skill_paths() -> Vec<PathBuf> {
    vec![
        PathBuf::from("skills/github-ops-tone/SKILL.md"),
        PathBuf::from("skills/ci-triage/SKILL.md"),
    ]
}

pub fn default_workflow_skill_paths(workflow_id: &str) -> Vec<PathBuf> {
    match workflow_id {
        "daily-work" => vec![
            PathBuf::from("skills/ci-triage/SKILL.md"),
            PathBuf::from("skills/digest-style/SKILL.md"),
        ],
        "merge-health" | "release-duty" => {
            vec![PathBuf::from("skills/pr-merge/SKILL.md")]
        }
        "review-radar" => vec![
            PathBuf::from("skills/pr-merge/SKILL.md"),
            PathBuf::from("skills/digest-style/SKILL.md"),
        ],
        "main-guard" | "my-pr-brief" | "comment-assist" => {
            vec![PathBuf::from("skills/ci-triage/SKILL.md")]
        }
        "oncall-handoff" | "release-notes" | "security-digest" => {
            vec![PathBuf::from("skills/digest-style/SKILL.md")]
        }
        _ => Vec::new(),
    }
}

pub fn load_chat_prompt_bundle(
    agent_path: &str,
    skill_paths: &[PathBuf],
    tools_doc: String,
    runtime_context: String,
) -> Result<PromptBundle> {
    let agent_path = if agent_path.is_empty() {
        default_chat_agent_path()
    } else {
        PathBuf::from(agent_path)
    };
    let agent = super::skill::load_agent(&agent_path)?;
    let skills = if !skill_paths.is_empty() {
        load_skills(skill_paths)?
    } else if !agent.skill_refs.is_empty() {
        load_skills_from_refs(&agent.skill_refs)?
    } else {
        load_skills(&default_chat_skill_paths())?
    };
    Ok(PromptBundle {
        agent,
        skills,
        tools_doc,
        runtime_context,
    })
}

pub fn load_workflow_spec(
    workflow_id: &str,
    agent_path: &PathBuf,
    skill_paths: &[PathBuf],
) -> Result<WorkflowSpec> {
    let agent = super::skill::load_agent_with_base(agent_path)?;
    let skills = if !skill_paths.is_empty() {
        load_skills(skill_paths)?
    } else if !agent.skill_refs.is_empty() {
        load_skills_from_refs(&agent.skill_refs)?
    } else {
        let paths = default_workflow_skill_paths(workflow_id);
        if paths.is_empty() {
            Vec::new()
        } else {
            load_skills(&paths)?
        }
    };
    Ok(WorkflowSpec { agent, skills })
}

pub fn load_classify_skills_for_triage(explicit: &[PathBuf]) -> Result<Vec<SkillSpec>> {
    if explicit.is_empty() {
        load_classify_skills_from_refs(&[])
    } else {
        load_skills(explicit)
    }
}

pub fn load_classify_skills_from_refs(refs: &[String]) -> Result<Vec<SkillSpec>> {
    if refs.is_empty() {
        load_skills(&[PathBuf::from("skills/ci-triage/SKILL.md")])
    } else {
        load_skills_from_refs(refs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::skill::{AgentSpec, SkillSpec};

    #[test]
    fn compose_system_prompt_includes_sections() {
        let bundle = PromptBundle {
            agent: AgentSpec {
                name: "chat".into(),
                description: String::new(),
                body: "Agent body".into(),
                skill_refs: vec![],
            },
            skills: vec![SkillSpec {
                name: "tone".into(),
                description: String::new(),
                body: "Be concise".into(),
                skill_refs: vec![],
            }],
            tools_doc: "tool_a".into(),
            runtime_context: "repos: x".into(),
        };
        let out = compose_system_prompt(&bundle);
        assert!(out.contains("Agent body"));
        assert!(out.contains("## Techniques"));
        assert!(out.contains("Be concise"));
        assert!(out.contains("## Tools"));
        assert!(out.contains("tool_a"));
        assert!(out.contains("## Context"));
    }

    #[test]
    fn compose_classify_prompt_uses_skills_not_verdict_duplication() {
        let skills = vec![SkillSpec {
            name: "ci-triage".into(),
            description: String::new(),
            body: "- flaky: transient\n- real: code bug".into(),
            skill_refs: vec![],
        }];
        let out = compose_classify_prompt("", &skills, None);
        assert!(out.contains("flaky: transient"));
        assert!(out.contains("reason: one line"));
    }
}
