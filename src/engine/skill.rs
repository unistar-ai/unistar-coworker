use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::{CoworkerError, Result};

const BASE_TOOLS_PATH: &str = "skills/_base/TOOLS.md";

#[derive(Debug, Clone, Default)]
pub struct MarkdownSpec {
    pub name: String,
    pub description: String,
    pub body: String,
    /// Technique skill names from frontmatter `skills:` (agents only).
    pub skill_refs: Vec<String>,
    /// Business/harness tool names from frontmatter `tools:` (skills only).
    pub tool_refs: Vec<String>,
    /// When `always: true` in SKILL.md frontmatter (used by intent routing tests).
    #[allow(dead_code)]
    pub always_load: bool,
    /// Lazy routing: substring triggers in the user message (skills only).
    pub intent_keywords: Vec<String>,
    /// Lazy routing: multi-word phrase triggers (skills only).
    #[allow(dead_code)]
    pub intent_phrases: Vec<String>,
    /// Extra score when these substrings appear (e.g. PR context for ci-triage).
    #[allow(dead_code)]
    pub intent_bonus_keywords: Vec<String>,
    /// Subtract `intent_penalty` when any of these appear (e.g. PR on branch-health skill).
    #[allow(dead_code)]
    pub intent_penalty_keywords: Vec<String>,
    /// Subtract `intent_penalty` when any phrase appears (e.g. "waiting for review").
    #[allow(dead_code)]
    pub intent_penalty_phrases: Vec<String>,
    /// Points subtracted on penalty keyword/phrase hit (default 6 in routing).
    #[allow(dead_code)]
    pub intent_penalty: i32,
}

pub type AgentSpec = MarkdownSpec;
pub type SkillSpec = MarkdownSpec;

#[derive(Debug, Default, Deserialize)]
struct SpecFrontmatter {
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    skills: Vec<String>,
    #[serde(default)]
    tools: Vec<String>,
    #[serde(default)]
    always: bool,
    #[serde(default)]
    intent_keywords: Vec<String>,
    #[serde(default)]
    intent_phrases: Vec<String>,
    #[serde(default)]
    intent_bonus_keywords: Vec<String>,
    #[serde(default)]
    intent_penalty_keywords: Vec<String>,
    #[serde(default)]
    intent_penalty_phrases: Vec<String>,
    #[serde(default)]
    intent_penalty: i32,
}

pub fn read_base_tools() -> String {
    std::fs::read_to_string(BASE_TOOLS_PATH).unwrap_or_else(|_| String::new())
}

pub fn with_base_tools(body: &str) -> String {
    let base = read_base_tools();
    if base.trim().is_empty() {
        return body.trim().to_string();
    }
    format!("{}\n\n## Base tools\n\n{}", body.trim(), base.trim())
}

/// Skill body for prompts and skill_load — omit tool-chain playbooks.
pub fn skill_body_for_prompt(body: &str) -> String {
    let marker = "## Tool chains";
    let Some(start) = body.find(marker) else {
        return body.trim().to_string();
    };
    let rest = &body[start + marker.len()..];
    let end = rest
        .find("\n## ")
        .map(|i| start + marker.len() + i)
        .unwrap_or(body.len());
    format!("{}{}", &body[..start], &body[end..])
        .trim()
        .to_string()
}

pub fn load_skill(path: impl AsRef<Path>) -> Result<SkillSpec> {
    load_markdown_spec(path)
}

pub fn load_skill_with_base(path: impl AsRef<Path>) -> Result<SkillSpec> {
    let mut spec = load_skill(path)?;
    spec.body = with_base_tools(&spec.body);
    Ok(spec)
}

pub fn load_agent(path: impl AsRef<Path>) -> Result<AgentSpec> {
    load_markdown_spec(path)
}

pub fn load_markdown_spec(path: impl AsRef<Path>) -> Result<MarkdownSpec> {
    let path = path.as_ref();
    let raw = std::fs::read_to_string(path)
        .map_err(|e| CoworkerError::Workflow(format!("read spec {}: {e}", path.display())))?;
    parse_markdown_spec(&raw)
}

pub fn parse_markdown_spec(raw: &str) -> Result<MarkdownSpec> {
    let trimmed = raw.trim_start();
    if !trimmed.starts_with("---") {
        return Ok(empty_markdown_spec(raw.to_string()));
    }

    let rest = trimmed.strip_prefix("---").unwrap_or(trimmed).trim_start();
    let Some((front, body)) = rest.split_once("\n---") else {
        return Ok(empty_markdown_spec(raw.to_string()));
    };

    let meta: SpecFrontmatter = serde_yaml::from_str(front.trim()).unwrap_or_default();

    Ok(MarkdownSpec {
        name: meta.name,
        description: meta.description,
        body: body.trim_start_matches('\n').trim().to_string(),
        skill_refs: meta.skills,
        tool_refs: meta.tools,
        always_load: meta.always,
        intent_keywords: meta.intent_keywords,
        intent_phrases: meta.intent_phrases,
        intent_bonus_keywords: meta.intent_bonus_keywords,
        intent_penalty_keywords: meta.intent_penalty_keywords,
        intent_penalty_phrases: meta.intent_penalty_phrases,
        intent_penalty: meta.intent_penalty,
    })
}

fn empty_markdown_spec(body: String) -> MarkdownSpec {
    MarkdownSpec {
        name: String::new(),
        description: String::new(),
        body,
        skill_refs: Vec::new(),
        tool_refs: Vec::new(),
        always_load: false,
        intent_keywords: Vec::new(),
        intent_phrases: Vec::new(),
        intent_bonus_keywords: Vec::new(),
        intent_penalty_keywords: Vec::new(),
        intent_penalty_phrases: Vec::new(),
        intent_penalty: 0,
    }
}

pub fn resolve_skill_ref(name: &str) -> PathBuf {
    let trimmed = name.trim().trim_end_matches(".md");
    if trimmed.contains('/') {
        if trimmed.ends_with("SKILL.md") {
            PathBuf::from(trimmed)
        } else {
            PathBuf::from(format!("{trimmed}/SKILL.md"))
        }
    } else {
        PathBuf::from(format!("skills/{trimmed}/SKILL.md"))
    }
}

pub fn load_skills(paths: &[PathBuf]) -> Result<Vec<SkillSpec>> {
    paths.iter().map(|p| load_skill(p.as_path())).collect()
}

pub fn load_skills_from_refs(refs: &[String]) -> Result<Vec<SkillSpec>> {
    refs.iter()
        .map(|r| load_skill(resolve_skill_ref(r)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_frontmatter() {
        let raw = "---\nname: daily-work\ndescription: triage\n---\n\n# Body\n";
        let s = parse_markdown_spec(raw).unwrap();
        assert_eq!(s.name, "daily-work");
        assert_eq!(s.description, "triage");
        assert!(s.body.contains("# Body"));
    }

    #[test]
    fn parses_skills_array_in_frontmatter() {
        let raw = "---\nname: daily-work\nskills: [ci-triage, digest-style]\n---\n\nBody\n";
        let s = parse_markdown_spec(raw).unwrap();
        assert_eq!(s.skill_refs, vec!["ci-triage", "digest-style"]);
    }

    #[test]
    fn parses_tools_and_always_in_frontmatter() {
        let raw = "---\nname: ci-triage\nalways: false\ntools: [pr_get_ci_snapshot, ci_get_failure_digest]\n---\n\nBody\n";
        let s = parse_markdown_spec(raw).unwrap();
        assert_eq!(
            s.tool_refs,
            vec!["pr_get_ci_snapshot", "ci_get_failure_digest"]
        );
        assert!(!s.always_load);
    }

    #[test]
    fn parses_always_load_skill() {
        let raw = "---\nname: github-ops-tone\nalways: true\n---\n\nBody\n";
        let s = parse_markdown_spec(raw).unwrap();
        assert!(s.always_load);
    }

    #[test]
    fn parses_intent_metadata_in_frontmatter() {
        let raw = "---\nname: ci-triage\nintent_keywords: [ci, fail]\nintent_phrases: [why is ci]\nintent_bonus_keywords: [pr]\nintent_penalty_keywords: [draft]\nintent_penalty: 4\n---\n\nBody\n";
        let s = parse_markdown_spec(raw).unwrap();
        assert_eq!(s.intent_keywords, vec!["ci", "fail"]);
        assert_eq!(s.intent_phrases, vec!["why is ci"]);
        assert_eq!(s.intent_bonus_keywords, vec!["pr"]);
        assert_eq!(s.intent_penalty_keywords, vec!["draft"]);
        assert_eq!(s.intent_penalty, 4);
    }

    #[test]
    fn resolve_skill_ref_short_name() {
        assert_eq!(
            resolve_skill_ref("ci-triage"),
            PathBuf::from("skills/ci-triage/SKILL.md")
        );
    }

    #[test]
    fn load_skill_with_base_appends_tools_section() {
        let spec = load_skill_with_base("skills/ci-triage/SKILL.md").unwrap();
        assert!(spec.body.contains("## Base tools"));
        assert!(spec.body.contains("Verdicts") || spec.body.contains("flaky"));
    }

    #[test]
    fn skill_body_for_prompt_strips_tool_chains() {
        let body = "## Tool chains\n| a | b |\n\n## Verdicts\n- flaky";
        let out = skill_body_for_prompt(body);
        assert!(!out.contains("Tool chains"));
        assert!(out.contains("Verdicts"));
    }
}
