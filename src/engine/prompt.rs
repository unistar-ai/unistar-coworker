use std::path::PathBuf;

use crate::config::ChatToolMode;
use crate::error::Result;

use super::skill::{load_skills, load_skills_from_refs, skill_body_for_prompt, PromptSpec, SkillSpec};
use super::skill_routing::SkillRegistry;

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

/// Prefix for the per-session runtime block (injected as a user message, not system).
pub const SESSION_CONTEXT_PREFIX: &str = "[session context]";

#[derive(Debug, Clone)]
pub struct PromptBundle {
    pub chat_prompt: PromptSpec,
    pub skills: Vec<SkillSpec>,
    /// Lazy chat: all skill names + descriptions (model picks `skill_load` from this list).
    pub skill_catalog: String,
    pub tools_doc: String,
    pub runtime_context: String,
}

#[derive(Debug, Clone)]
pub struct WorkflowSpec {
    pub id: String,
    pub description: String,
    pub skills: Vec<SkillSpec>,
}

/// Omit tool-chain playbooks from skill bodies — the model discovers tools itself.
fn join_skills(skills: &[SkillSpec]) -> String {
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
            format!("### {title}\n{}", skill_body_for_prompt(&s.body))
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub fn compose_static_system_prompt(bundle: &PromptBundle) -> String {
    let mut parts = vec![bundle.chat_prompt.body.clone()];
    if !bundle.skill_catalog.trim().is_empty() {
        parts.push(format!(
            "## Available skills\n\n{}",
            bundle.skill_catalog.trim()
        ));
    }
    let techniques = join_skills(&bundle.skills);
    if !techniques.is_empty() {
        parts.push(format!("## Techniques\n{techniques}"));
    }
    if !bundle.tools_doc.trim().is_empty() {
        parts.push(format!("## Tools\n{}", bundle.tools_doc.trim()));
    }
    parts.join("\n\n")
}

/// Chat system prompt — static body from `prompts/`, skill catalog, techniques; native mode adds a one-line hint.
pub fn compose_chat_system_prompt(bundle: &PromptBundle, tool_mode: ChatToolMode) -> String {
    let mut out = compose_static_system_prompt(bundle);
    if matches!(tool_mode, ChatToolMode::Native) {
        out.push_str("\n\nUse the native tool schemas attached to this request.");
    }
    out
}

/// Full system prompt including runtime (legacy). Prefer [`compose_chat_system_prompt`]
/// plus [`format_session_context_message`] for chat sessions.
#[allow(dead_code)]
pub fn compose_system_prompt(bundle: &PromptBundle) -> String {
    let static_part = compose_static_system_prompt(bundle);
    if bundle.runtime_context.trim().is_empty() {
        return static_part;
    }
    format!(
        "{static_part}\n\n## Context\n{}",
        bundle.runtime_context.trim()
    )
}

pub fn format_session_context_message(runtime_context: &str) -> String {
    let trimmed = runtime_context.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    format!("{SESSION_CONTEXT_PREFIX}\n{trimmed}")
}

pub fn compose_classify_prompt(playbook_prefix: &str, skills: &[SkillSpec]) -> String {
    let mut parts = Vec::new();
    if !playbook_prefix.is_empty() {
        parts.push(playbook_prefix.trim().to_string());
    }
    let techniques = join_skills(skills);
    if !techniques.is_empty() {
        parts.push(techniques);
    }
    parts.push(CLASSIFY_OUTPUT_CONTRACT.to_string());
    parts.join("\n\n")
}

pub fn default_chat_prompt_path() -> PathBuf {
    PathBuf::from("prompts/chat.md")
}

pub fn default_chat_skill_paths() -> Vec<PathBuf> {
    vec![
        PathBuf::from("skills/github-ops-tone/SKILL.md"),
        PathBuf::from("skills/ci-triage/SKILL.md"),
    ]
}

pub fn load_chat_prompt_bundle_for_session(
    prompt_path: &str,
    skill_paths: &[PathBuf],
    tools_doc: String,
    runtime_context: String,
    user_message: &str,
    lazy_skills: bool,
) -> Result<(PromptBundle, SkillRegistry)> {
    let registry = SkillRegistry::load_for_chat(prompt_path, skill_paths)?;
    let skills = registry.select_for_message(user_message, lazy_skills);
    let skill_catalog = if lazy_skills {
        registry.format_catalog()
    } else {
        String::new()
    };
    let prompt_path = if prompt_path.is_empty() {
        default_chat_prompt_path()
    } else {
        PathBuf::from(prompt_path)
    };
    let chat_prompt = super::skill::load_prompt(&prompt_path)?;
    Ok((
        PromptBundle {
            chat_prompt,
            skills,
            skill_catalog,
            tools_doc,
            runtime_context,
        },
        registry,
    ))
}

pub fn load_workflow_spec(workflow_id: &str, skill_paths: &[PathBuf]) -> Result<WorkflowSpec> {
    let meta = super::workflow_registry::require(workflow_id)?;
    let skills = if !skill_paths.is_empty() {
        load_skills(skill_paths)?
    } else {
        let paths = super::workflow_registry::default_skill_paths(workflow_id);
        if paths.is_empty() {
            Vec::new()
        } else {
            load_skills(&paths)?
        }
    };
    Ok(WorkflowSpec {
        id: workflow_id.to_string(),
        description: meta.description.to_string(),
        skills,
    })
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
    use crate::engine::skill::{PromptSpec, SkillSpec};

    #[test]
    fn compose_static_system_prompt_omits_runtime_context() {
        let bundle = PromptBundle {
            chat_prompt: PromptSpec {
                name: "chat".into(),
                description: String::new(),
                body: "Prompt body".into(),
                skill_refs: vec![],
                tool_refs: vec![],
                always_load: false,
                intent_keywords: vec![],
                intent_phrases: vec![],
                intent_bonus_keywords: vec![],
                intent_penalty_keywords: vec![],
                intent_penalty_phrases: vec![],
                intent_penalty: 0,
            },
            skills: vec![],
            skill_catalog: String::new(),
            tools_doc: String::new(),
            runtime_context: "repos: changed-store".into(),
        };
        let static_out = compose_static_system_prompt(&bundle);
        assert!(static_out.contains("Prompt body"));
        assert!(!static_out.contains("changed-store"));
        assert!(!static_out.contains("## Context"));

        let mut bundle_b = bundle.clone();
        bundle_b.runtime_context = "repos: other-store".into();
        assert_eq!(compose_static_system_prompt(&bundle), compose_static_system_prompt(&bundle_b));
    }

    #[test]
    fn format_session_context_message_prefix() {
        let msg = format_session_context_message("repos: x");
        assert!(msg.starts_with(SESSION_CONTEXT_PREFIX));
        assert!(msg.contains("repos: x"));
    }

    #[test]
    fn compose_system_prompt_includes_sections() {
        let bundle = PromptBundle {
            chat_prompt: PromptSpec {
                name: "chat".into(),
                description: String::new(),
                body: "Prompt body".into(),
                skill_refs: vec![],
                tool_refs: vec![],
                always_load: false,
                intent_keywords: vec![],
                intent_phrases: vec![],
                intent_bonus_keywords: vec![],
                intent_penalty_keywords: vec![],
                intent_penalty_phrases: vec![],
                intent_penalty: 0,
            },
            skills: vec![SkillSpec {
                name: "tone".into(),
                description: String::new(),
                body: "Be concise".into(),
                skill_refs: vec![],
                tool_refs: vec![],
                always_load: false,
                intent_keywords: vec![],
                intent_phrases: vec![],
                intent_bonus_keywords: vec![],
                intent_penalty_keywords: vec![],
                intent_penalty_phrases: vec![],
                intent_penalty: 0,
            }],
            skill_catalog: String::new(),
            tools_doc: "tool_a".into(),
            runtime_context: "repos: x".into(),
        };
        let out = compose_system_prompt(&bundle);
        assert!(out.contains("Prompt body"));
        assert!(out.contains("## Techniques"));
        assert!(out.contains("Be concise"));
        assert!(out.contains("## Tools"));
        assert!(out.contains("tool_a"));
        assert!(out.contains("## Context"));
        let static_only = compose_static_system_prompt(&bundle);
        assert!(!static_only.contains("## Context"));
    }

    #[test]
    fn compose_system_prompt_omits_empty_tools_section() {
        let bundle = PromptBundle {
            chat_prompt: PromptSpec {
                name: "chat".into(),
                description: String::new(),
                body: "Prompt body".into(),
                skill_refs: vec![],
                tool_refs: vec![],
                always_load: false,
                intent_keywords: vec![],
                intent_phrases: vec![],
                intent_bonus_keywords: vec![],
                intent_penalty_keywords: vec![],
                intent_penalty_phrases: vec![],
                intent_penalty: 0,
            },
            skills: vec![],
            skill_catalog: String::new(),
            tools_doc: String::new(),
            runtime_context: "repos: x".into(),
        };
        let out = compose_system_prompt(&bundle);
        assert!(!out.contains("## Tools"));
        assert!(out.contains("## Context"));
    }

    #[test]
    fn load_chat_prompt_bundle_eager_loads_all_skills() {
        let (bundle, _) = load_chat_prompt_bundle_for_session(
            "",
            &[],
            String::new(),
            String::new(),
            "",
            false,
        )
        .unwrap();
        assert!(bundle.skills.len() >= 5);
        let names: Vec<_> = bundle.skills.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"code-edit"));
        assert!(names.contains(&"debug"));
    }

    #[test]
    fn lazy_session_loads_always_load_skills_only() {
        let (bundle, _) = load_chat_prompt_bundle_for_session(
            "",
            &[],
            String::new(),
            String::new(),
            "fix this bug and run cargo test",
            true,
        )
        .unwrap();
        assert_eq!(bundle.skills.len(), 1);
        assert_eq!(bundle.skills[0].name, "github-ops-tone");
    }

    #[test]
    fn lazy_session_loads_skill_catalog() {
        let (bundle, _) = load_chat_prompt_bundle_for_session(
            "",
            &[],
            String::new(),
            String::new(),
            "fix this bug and run cargo test",
            true,
        )
        .unwrap();
        assert!(bundle.skill_catalog.contains("**code-edit**"));
        assert!(bundle.skill_catalog.contains("**github-ops-tone**"));
        let static_prompt = compose_static_system_prompt(&bundle);
        assert!(static_prompt.contains("## Available skills"));
        assert!(static_prompt.contains("## Techniques"));
        assert!(static_prompt.contains("github-ops-tone"));
    }

    #[test]
    fn join_skills_omits_tool_chains() {
        let skills = vec![SkillSpec {
            name: "ci-triage".into(),
            description: String::new(),
            body: "## Tool chains\n| x | y |\n\n## Rules\n- be careful".into(),
            skill_refs: vec![],
            tool_refs: vec![],
            always_load: false,
            intent_keywords: vec![],
            intent_phrases: vec![],
            intent_bonus_keywords: vec![],
            intent_penalty_keywords: vec![],
            intent_penalty_phrases: vec![],
            intent_penalty: 0,
        }];
        let out = join_skills(&skills);
        assert!(!out.contains("Tool chains"));
        assert!(out.contains("be careful"));
    }

    #[test]
    fn compose_classify_prompt_uses_skills_not_verdict_duplication() {
        let skills = vec![SkillSpec {
            name: "ci-triage".into(),
            description: String::new(),
            body: "- flaky: transient\n- real: code bug".into(),
            skill_refs: vec![],
            tool_refs: vec![],
            always_load: false,
            intent_keywords: vec![],
            intent_phrases: vec![],
            intent_bonus_keywords: vec![],
            intent_penalty_keywords: vec![],
            intent_penalty_phrases: vec![],
            intent_penalty: 0,
        }];
        let out = compose_classify_prompt("", &skills);
        assert!(out.contains("flaky: transient"));
        assert!(out.contains("reason: one line"));
    }
}
