use std::path::{Path, PathBuf};

use crate::config::ChatToolMode;
use crate::error::Result;

use super::skill::{skill_body_for_prompt, PromptSpec, SkillSpec};
use super::skill_routing::SkillRegistry;

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
    // NOTE: the "Available skills" catalog is NOT inlined here — it's injected
    // as a separate message in chat_loop so it isn't subject to
    // trim_system_content's budget cuts (the catalog can be long with many
    // skills, and blind truncation was cutting skill names mid-word).
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

pub fn default_chat_prompt_path() -> PathBuf {
    PathBuf::from("prompts/chat.md")
}

pub fn default_chat_skill_paths() -> Vec<PathBuf> {
    // Scan the skills/ directory for every SKILL.md so newly added skills are
    // automatically registered (and surfaced in the system prompt's "Available
    // skills" list). Previously this was a hardcoded 2-skill list, which meant
    // any skill not explicitly listed (e.g. my-prs) couldn't be loaded via
    // skill_load — the registry didn't know about it.
    scan_skill_dir(&crate::repo::resolve_repo_path("skills"))
}

/// Recursively collect every `SKILL.md` under `dir`, sorted for stable order.
/// Subdirectories whose name starts with `_` (e.g. `_base`) are skipped —
/// they hold shared fragments (TOOLS.md), not skills.
fn scan_skill_dir(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        // Directory missing (e.g. running outside the repo) — fall back to the
        // known defaults so the agent still has its core skills.
        return vec![
            PathBuf::from("skills/general-agent-tone/SKILL.md"),
            PathBuf::from("skills/code-edit/SKILL.md"),
        ];
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let path = e.path();
            let name = e.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with('_') {
                return None;
            }
            if path.is_dir() {
                let skill_md = path.join("SKILL.md");
                if skill_md.is_file() {
                    return Some(skill_md);
                }
            }
            None
        })
        .collect();
    paths.sort();
    paths
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
                argument_hint: String::new(),
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
        assert_eq!(
            compose_static_system_prompt(&bundle),
            compose_static_system_prompt(&bundle_b)
        );
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
                argument_hint: String::new(),
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
                argument_hint: String::new(),
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
                argument_hint: String::new(),
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
        let (bundle, _) =
            load_chat_prompt_bundle_for_session("", &[], String::new(), String::new(), "", false)
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
        assert_eq!(bundle.skills[0].name, "general-agent-tone");
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
        // Available skills is now a separate message, NOT in the static prompt.
        assert!(!static_prompt.contains("## Available skills"));
        assert!(static_prompt.contains("## Techniques"));
        assert!(static_prompt.contains("general-agent-tone"));
    }

    #[test]
    fn join_skills_omits_tool_chains() {
        let skills = vec![SkillSpec {
            name: "ci-triage".into(),
            description: String::new(),
            body: "## Tool chains\n| x | y |\n\n## Rules\n- be careful".into(),
            argument_hint: String::new(),
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
    fn default_chat_skill_paths_scans_skills_directory() {
        // The skills dir is scanned at runtime relative to the cargo test CWD
        // (project root). Verify my-prs and the known defaults are included —
        // this guards against regressing back to a hardcoded 2-skill list that
        // silently dropped every other skill from the registry.
        let paths = default_chat_skill_paths();
        let names: Vec<String> = paths
            .iter()
            .filter_map(|p| {
                p.parent()
                    .and_then(|d| d.file_name())
                    .map(|n| n.to_string_lossy().to_string())
            })
            .collect();
        assert!(
            names.contains(&"general-agent-tone".to_string()),
            "missing general-agent-tone"
        );
        assert!(
            names.contains(&"code-edit".to_string()),
            "missing code-edit"
        );
        assert!(
            names.contains(&"my-prs".to_string()),
            "missing my-prs — dir scan must include it"
        );
        // _base is not a skill (no SKILL.md) — must be excluded.
        assert!(
            !names.contains(&"_base".to_string()),
            "_base should be skipped"
        );
    }
}
