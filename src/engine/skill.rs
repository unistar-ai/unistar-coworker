use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::{CoworkerError, Result};

const BASE_TOOLS_PATH: &str = "skills/_base/TOOLS.md";

#[derive(Debug, Clone)]
pub struct MarkdownSpec {
    pub name: String,
    pub description: String,
    pub body: String,
    /// Technique skill names from frontmatter `skills:` (agents only).
    pub skill_refs: Vec<String>,
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

pub fn load_agent_with_base(path: impl AsRef<Path>) -> Result<AgentSpec> {
    let mut spec = load_agent(path)?;
    spec.body = with_base_tools(&spec.body);
    Ok(spec)
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
        return Ok(MarkdownSpec {
            name: String::new(),
            description: String::new(),
            body: raw.to_string(),
            skill_refs: Vec::new(),
        });
    }

    let rest = trimmed.strip_prefix("---").unwrap_or(trimmed).trim_start();
    let Some((front, body)) = rest.split_once("\n---") else {
        return Ok(MarkdownSpec {
            name: String::new(),
            description: String::new(),
            body: raw.to_string(),
            skill_refs: Vec::new(),
        });
    };

    let meta: SpecFrontmatter = serde_yaml::from_str(front.trim()).unwrap_or_default();

    Ok(MarkdownSpec {
        name: meta.name,
        description: meta.description,
        body: body.trim_start_matches('\n').trim().to_string(),
        skill_refs: meta.skills,
    })
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
}
