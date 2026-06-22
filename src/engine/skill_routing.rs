//! Skill registry: lazy loading, intent routing, and skill_search for chat.

use std::path::PathBuf;

#[cfg(test)]
use crate::agent::chat_discovery;
use crate::error::Result;

use super::skill::{
    load_agent, load_skills, load_skills_from_refs, SkillSpec,
};

/// All technique skills available to a chat session.
#[derive(Debug, Clone)]
pub struct SkillRegistry {
    pub skills: Vec<SkillSpec>,
}

#[cfg(test)]
impl SkillRegistry {
    pub fn from_skills(skills: Vec<SkillSpec>) -> Self {
        Self { skills }
    }
}

impl SkillRegistry {
    /// Load every skill referenced by the chat agent (or explicit config paths).
    pub fn load_for_chat(agent_path: &str, skill_paths: &[PathBuf]) -> Result<Self> {
        let agent_path = if agent_path.is_empty() {
            super::prompt::default_chat_agent_path()
        } else {
            PathBuf::from(agent_path)
        };
        let agent = load_agent(&agent_path)?;
        let skills = if !skill_paths.is_empty() {
            load_skills(skill_paths)?
        } else if !agent.skill_refs.is_empty() {
            load_skills_from_refs(&agent.skill_refs)?
        } else {
            load_skills(&super::prompt::default_chat_skill_paths())?
        };
        Ok(Self { skills })
    }

    pub fn get(&self, name: &str) -> Option<&SkillSpec> {
        let want = name.trim().to_ascii_lowercase();
        self.skills.iter().find(|s| skill_key(s) == want)
    }

    /// Skills to inject into the system prompt. Lazy/auto chat defers to `skill_search` + `skill_load`.
    pub fn select_for_message(&self, _message: &str, lazy: bool) -> Vec<SkillSpec> {
        if lazy {
            return Vec::new();
        }
        self.skills.clone()
    }

    /// Intent-based skill pick for unit tests (lazy chat uses `skill_search` + `skill_load`).
    #[cfg(test)]
    pub fn select_for_message_by_intent(&self, message: &str) -> Vec<SkillSpec> {
        let lower = message.to_ascii_lowercase();
        let mut selected: Vec<SkillSpec> = self
            .skills
            .iter()
            .filter(|s| s.always_load)
            .cloned()
            .collect();
        for skill in &self.skills {
            if skill.always_load {
                continue;
            }
            if skill_intent_score(skill, &lower) >= INTENT_THRESHOLD
                && !selected.iter().any(|s| s.name == skill.name)
            {
                selected.push(skill.clone());
            }
        }
        selected
    }

    /// Search skills by keyword (name, description, tools).
    pub fn search(&self, query: &str, limit: usize) -> Vec<&SkillSpec> {
        let terms: Vec<String> = query
            .to_ascii_lowercase()
            .split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|t| t.len() >= 2)
            .map(str::to_string)
            .collect();
        if terms.is_empty() {
            return Vec::new();
        }
        let limit = limit.clamp(1, 15);
        let mut scored: Vec<(&SkillSpec, i32)> = self
            .skills
            .iter()
            .map(|s| (s, score_skill_query(s, &terms)))
            .filter(|(_, score)| *score > 0)
            .collect();
        scored.sort_by_key(|b| std::cmp::Reverse(b.1));
        scored.into_iter().take(limit).map(|(s, _)| s).collect()
    }

    /// Union of `tools[]` from the given skills (deduped, catalog order preserved).
    pub fn collect_tool_refs(skills: &[SkillSpec]) -> Vec<String> {
        let mut out = Vec::new();
        for skill in skills {
            for tool in &skill.tool_refs {
                let t = tool.trim();
                if t.is_empty() {
                    continue;
                }
                if !out.iter().any(|x: &String| x == t) {
                    out.push(t.to_string());
                }
            }
        }
        out
    }

    pub fn format_search_results(matches: &[&SkillSpec]) -> String {
        if matches.is_empty() {
            return "No skills matched. Try keywords like ci, merge, digest, review.".into();
        }
        let mut lines = vec![format!("{} skill(s):", matches.len())];
        for s in matches {
            lines.push(format!(
                "[{}] {} — {}",
                s.name,
                s.name,
                trim_desc(&s.description),
            ));
        }
        lines.join("\n")
    }

    pub fn format_skill_load(skill: &SkillSpec) -> String {
        format!("### {}\n{}", skill.name, crate::engine::skill::skill_body_for_prompt(&skill.body))
    }
}

fn skill_key(skill: &SkillSpec) -> String {
    if skill.name.is_empty() {
        String::new()
    } else {
        skill.name.to_ascii_lowercase()
    }
}

fn trim_desc(desc: &str) -> &str {
    desc.trim()
}

fn score_skill_query(skill: &SkillSpec, terms: &[String]) -> i32 {
    let name = skill.name.to_ascii_lowercase();
    let desc = skill.description.to_ascii_lowercase();
    let body = skill.body.to_ascii_lowercase();
    let mut score = 0;
    for term in terms {
        if name.contains(term) {
            score += 12;
        }
        if desc.contains(term) {
            score += 8;
        }
        if body.contains(term) {
            score += 3;
        }
        for tool in &skill.tool_refs {
            if tool.to_ascii_lowercase().contains(term) {
                score += 5;
            }
        }
        for kw in &skill.intent_keywords {
            if kw.to_ascii_lowercase().contains(term) {
                score += 4;
            }
        }
    }
    score
}

#[cfg(test)]
const INTENT_KEYWORD_SCORE: i32 = 10;
#[cfg(test)]
const INTENT_PHRASE_SCORE: i32 = 14;
#[cfg(test)]
const INTENT_BONUS_SCORE: i32 = 6;
#[cfg(test)]
const INTENT_THRESHOLD: i32 = 8;
#[cfg(test)]
const DEFAULT_INTENT_PENALTY: i32 = 6;

#[cfg(test)]
fn skill_intent_score(skill: &SkillSpec, lower: &str) -> i32 {
    if skill.intent_keywords.is_empty()
        && skill.intent_phrases.is_empty()
        && skill.intent_bonus_keywords.is_empty()
    {
        return generic_intent_score(lower);
    }
    score_from_intent_metadata(skill, lower)
}

#[cfg(test)]
fn score_from_intent_metadata(skill: &SkillSpec, lower: &str) -> i32 {
    let mut score = 0i32;
    for kw in &skill.intent_keywords {
        let kw = kw.trim().to_ascii_lowercase();
        if !kw.is_empty() && lower.contains(&kw) {
            score += INTENT_KEYWORD_SCORE;
        }
    }
    for phrase in &skill.intent_phrases {
        let phrase = phrase.trim().to_ascii_lowercase();
        if !phrase.is_empty() && lower.contains(&phrase) {
            score += INTENT_PHRASE_SCORE;
        }
    }
    for bonus in &skill.intent_bonus_keywords {
        let bonus = bonus.trim().to_ascii_lowercase();
        if bonus == "#" {
            if lower.contains('#')
                || chat_discovery::extract_pr_number_for_autofill(lower).is_some()
            {
                score += INTENT_BONUS_SCORE;
            }
        } else if !bonus.is_empty() && lower.contains(&bonus) {
            score += INTENT_BONUS_SCORE;
        }
    }
    let penalty = if skill.intent_penalty > 0 {
        skill.intent_penalty
    } else {
        DEFAULT_INTENT_PENALTY
    };
    for pk in &skill.intent_penalty_keywords {
        let pk = pk.trim().to_ascii_lowercase();
        if !pk.is_empty() && lower.contains(&pk) {
            score = score.saturating_sub(penalty);
        }
    }
    for pp in &skill.intent_penalty_phrases {
        let pp = pp.trim().to_ascii_lowercase();
        if !pp.is_empty() && lower.contains(&pp) {
            score = score.saturating_sub(penalty);
        }
    }
    score
}

#[cfg(test)]
fn generic_intent_score(lower: &str) -> i32 {
    if lower.len() < 8 {
        0
    } else {
        4
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_registry() -> SkillRegistry {
        SkillRegistry::from_skills(vec![
                SkillSpec {
                    name: "github-ops-tone".into(),
                    description: "Secretary tone".into(),
                    body: "Be accurate".into(),
                    skill_refs: vec![],
                    tool_refs: vec![],
                    always_load: true,
                    intent_keywords: vec![],
                    intent_phrases: vec![],
                    intent_bonus_keywords: vec![],
                    intent_penalty_keywords: vec![],
                    intent_penalty_phrases: vec![],
                    intent_penalty: 0,
                },
                SkillSpec {
                    name: "ci-triage".into(),
                    description: "Classify CI failures".into(),
                    body: "Tool chains here".into(),
                    skill_refs: vec![],
                    tool_refs: vec!["pr_get_ci_snapshot".into()],
                    always_load: false,
                    intent_keywords: vec!["ci".into(), "fail".into(), "test".into()],
                    intent_phrases: vec![],
                    intent_bonus_keywords: vec!["pr".into(), "#".into()],
                    intent_penalty_keywords: vec![],
                    intent_penalty_phrases: vec![],
                    intent_penalty: 0,
                },
                SkillSpec {
                    name: "pr-merge".into(),
                    description: "Merge blockers".into(),
                    body: "Blockers".into(),
                    skill_refs: vec![],
                    tool_refs: vec!["pr_get_merge_blockers".into()],
                    always_load: false,
                    intent_keywords: vec!["merge".into(), "approve".into()],
                    intent_phrases: vec!["needs to approve".into()],
                    intent_bonus_keywords: vec![],
                    intent_penalty_keywords: vec![],
                    intent_penalty_phrases: vec![],
                    intent_penalty: 0,
                },
            ])
    }

    #[test]
    fn lazy_select_returns_empty() {
        let reg = sample_registry();
        let picked = reg.select_for_message("Why is CI failing on PR #42?", true);
        assert!(picked.is_empty());
    }

    #[test]
    fn intent_select_always_plus_ci() {
        let reg = sample_registry();
        let picked = reg.select_for_message_by_intent("Why is CI failing on PR #42?");
        let names: Vec<_> = picked.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"github-ops-tone"));
        assert!(names.contains(&"ci-triage"));
        assert!(!names.contains(&"pr-merge"));
    }

    #[test]
    fn intent_select_merge_review() {
        let reg = sample_registry();
        let picked = reg.select_for_message_by_intent("Who needs to approve before merge?");
        let names: Vec<_> = picked.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"pr-merge"));
    }

    #[test]
    fn native_mode_loads_all() {
        let reg = sample_registry();
        assert_eq!(reg.select_for_message("hello", false).len(), 3);
    }

    #[test]
    fn search_finds_ci_triage() {
        let reg = sample_registry();
        let hits = reg.search("ci failure", 5);
        assert!(hits.iter().any(|s| s.name == "ci-triage"));
    }

    #[test]
    fn intent_select_issue_tracker() {
        let reg = SkillRegistry::from_skills(vec![SkillSpec {
            name: "issue-tracker".into(),
            description: "issues".into(),
            body: String::new(),
            skill_refs: vec![],
            tool_refs: vec![],
            always_load: false,
            intent_keywords: vec!["issue".into(), "bug".into()],
            intent_phrases: vec![],
            intent_bonus_keywords: vec![],
            intent_penalty_keywords: vec![],
            intent_penalty_phrases: vec![],
            intent_penalty: 0,
        }]);
        let picked = reg.select_for_message_by_intent("list open bugs in the repo");
        assert!(picked.iter().any(|s| s.name == "issue-tracker"));
    }

    #[test]
    fn intent_select_security() {
        let reg = SkillRegistry::from_skills(vec![SkillSpec {
            name: "security-alerts".into(),
            description: "dependabot".into(),
            body: String::new(),
            skill_refs: vec![],
            tool_refs: vec![],
            always_load: false,
            intent_keywords: vec!["dependabot".into(), "security".into()],
            intent_phrases: vec![],
            intent_bonus_keywords: vec![],
            intent_penalty_keywords: vec![],
            intent_penalty_phrases: vec![],
            intent_penalty: 0,
        }]);
        let picked = reg.select_for_message_by_intent("show dependabot vulnerabilities");
        assert!(picked.iter().any(|s| s.name == "security-alerts"));
    }

    #[test]
    fn search_finds_release_skill() {
        let reg = SkillRegistry::from_skills(vec![SkillSpec {
            name: "release-backport".into(),
            description: "backport merged PRs".into(),
            body: String::new(),
            skill_refs: vec![],
            tool_refs: vec!["pr_list_backport_candidates".into()],
            always_load: false,
            intent_keywords: vec!["backport".into()],
            intent_phrases: vec![],
            intent_bonus_keywords: vec![],
            intent_penalty_keywords: vec![],
            intent_penalty_phrases: vec![],
            intent_penalty: 0,
        }]);
        let hits = reg.search("backport release", 3);
        assert!(hits.iter().any(|s| s.name == "release-backport"));
    }

    #[test]
    fn intent_ci_health_penalizes_pr_questions() {
        let skill = super::super::skill::load_skill("skills/ci-health/SKILL.md").unwrap();
        let reg = SkillRegistry::from_skills(vec![skill]);
        let picked = reg.select_for_message_by_intent("Why is CI failing on PR #42?");
        assert!(!picked.iter().any(|s| s.name == "ci-health"));
        let branch = reg.select_for_message_by_intent("How is main branch CI health?");
        assert!(branch.iter().any(|s| s.name == "ci-health"));
    }
}
