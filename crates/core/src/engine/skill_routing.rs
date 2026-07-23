//! Skill registry: lazy loading, skill catalog in system prompt, and skill_load for chat.

use std::collections::HashSet;
use std::path::PathBuf;

use crate::agent::chat_discovery;
use crate::error::Result;

use super::skill::{load_prompt, load_skills, SkillSpec};

const INTENT_KEYWORD_SCORE: i32 = 10;
const INTENT_PHRASE_SCORE: i32 = 14;
const INTENT_BONUS_SCORE: i32 = 6;
const INTENT_THRESHOLD: i32 = 8;
const DEFAULT_INTENT_PENALTY: i32 = 6;

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
    /// Load skills for the chat system prompt.
    ///
    /// Skill resolution order:
    /// 1. Explicit `skill_paths` from config (full override).
    /// 2. Otherwise, scan the `skills/` directory for every `SKILL.md` — this
    ///    is the default. No need to list skills in the prompt frontmatter;
    ///    anything dropped into `skills/` is automatically registered and
    ///    surfaced in the "Available skills" catalog (loadable via skill_load).
    ///
    /// In lazy mode only `always: true` skills are injected into the prompt
    /// (## Techniques); the rest appear in the catalog and are loaded on demand.
    pub fn load_for_chat(prompt_path: &str, skill_paths: &[PathBuf]) -> Result<Self> {
        let prompt_path = if prompt_path.is_empty() {
            super::prompt::default_chat_prompt_path()
        } else {
            PathBuf::from(prompt_path)
        };
        let _chat_prompt = load_prompt(&prompt_path)?;
        let skills = if !skill_paths.is_empty() {
            load_skills(skill_paths)?
        } else {
            load_skills(&super::prompt::default_chat_skill_paths())?
        };
        Ok(Self { skills })
    }

    pub fn get(&self, name: &str) -> Option<&SkillSpec> {
        let want = name.trim().to_ascii_lowercase();
        self.skills.iter().find(|s| skill_key(s) == want)
    }

    /// Lazy chat: `always_load` skills plus intent-matched techniques for this user message.
    pub fn select_for_message(&self, message: &str, lazy: bool) -> Vec<SkillSpec> {
        if lazy {
            return self.select_lazy_skills(message);
        }
        self.skills.clone()
    }

    /// Same as lazy [`select_for_message`](Self::select_for_message) (tests / diagnostics).
    pub fn select_for_message_by_intent(&self, message: &str) -> Vec<SkillSpec> {
        self.select_lazy_skills(message)
    }

    fn select_lazy_skills(&self, message: &str) -> Vec<SkillSpec> {
        let lower = message.trim().to_ascii_lowercase();
        let mut selected = self.always_load_skills();
        if lower.is_empty() {
            return selected;
        }
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

    /// Skills with `always: true` in frontmatter — injected under ## Techniques in lazy chat.
    pub fn always_load_skills(&self) -> Vec<SkillSpec> {
        self.skills
            .iter()
            .filter(|s| s.always_load)
            .cloned()
            .collect()
    }

    /// Name + description catalog for lazy chat (## Available skills in system prompt).
    pub fn format_catalog(&self) -> String {
        if self.skills.is_empty() {
            return String::new();
        }
        let mut lines = vec![
            "Use **skill_load** with the skill `name` when a task matches.".into(),
            String::new(),
        ];
        for s in &self.skills {
            let desc = trim_desc(&s.description);
            if desc.is_empty() {
                lines.push(format!("- **{}**", s.name));
            } else {
                lines.push(format!("- **{}** — {}", s.name, desc));
            }
        }
        lines.join("\n")
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

    pub fn format_skill_load(skill: &SkillSpec) -> String {
        format!(
            "### {}\n{}",
            skill.name,
            crate::engine::skill::skill_body_for_prompt(&skill.body)
        )
    }
}

fn skill_key(skill: &SkillSpec) -> String {
    if skill.name.is_empty() {
        String::new()
    } else {
        skill.name.to_ascii_lowercase()
    }
}

/// Append skills from `extra` into `base` that aren't already present (by
/// lowercased name), so a directory scan can add skills the prompt didn't list
/// without duplicating ones it did.
#[allow(dead_code)]
fn merge_skills(base: &mut Vec<SkillSpec>, extra: Vec<SkillSpec>) {
    let existing: std::collections::HashSet<String> = base.iter().map(skill_key).collect();
    for s in extra {
        let key = skill_key(&s);
        if !key.is_empty() && !existing.contains(&key) {
            base.push(s);
        }
    }
}

fn trim_desc(desc: &str) -> &str {
    desc.trim()
}

fn skill_intent_score(skill: &SkillSpec, lower: &str) -> i32 {
    if skill.intent_keywords.is_empty()
        && skill.intent_phrases.is_empty()
        && skill.intent_bonus_keywords.is_empty()
    {
        return generic_intent_score(lower);
    }
    score_from_intent_metadata(skill, lower)
}

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

fn generic_intent_score(lower: &str) -> i32 {
    if lower.len() < 8 {
        0
    } else {
        4
    }
}

/// Harness nudge when the model runs `gh` via `bash_run` without loading **gh-cli**.
pub fn nudge_if_gh_bash_without_gh_cli_skill(
    bash_commands: impl IntoIterator<Item = impl AsRef<str>>,
    loaded_skills: &HashSet<String>,
) -> Option<String> {
    if loaded_skills
        .iter()
        .any(|s| s.eq_ignore_ascii_case("gh-cli"))
    {
        return None;
    }
    for cmd in bash_commands {
        if bash_command_uses_gh_cli(cmd.as_ref()) {
            return Some(
                "This turn uses the GitHub CLI (`gh`) via **bash_run**, but **gh-cli** is not \
loaded. Call **skill_load** with `name: \"gh-cli\"` first (or use harness **pr_*** / **ci_*** \
tools when their schemas are available). Follow gh-cli for auth checks, `--json`, and \
non-interactive flags (`GH_PROMPT_DISABLED=1`)."
                    .into(),
            );
        }
    }
    None
}

fn bash_command_uses_gh_cli(command: &str) -> bool {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return false;
    }
    for line in trimmed.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        if line == "gh" || line.starts_with("gh ") || line.contains(" gh ") {
            return true;
        }
        if line.starts_with("gh\t") {
            return true;
        }
        for sep in ["&&", "||", "|", ";"] {
            for part in line.split(sep) {
                let part = part.trim();
                if part == "gh" || part.starts_with("gh ") {
                    return true;
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_registry() -> SkillRegistry {
        SkillRegistry::from_skills(vec![
            SkillSpec {
                name: "general-agent-tone".into(),
                description: "General agent tone".into(),
                body: "Be accurate".into(),
                argument_hint: String::new(),
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
                name: "github-ops-tone".into(),
                description: "GitHub ops tone".into(),
                body: "Ops style".into(),
                argument_hint: String::new(),
                skill_refs: vec![],
                tool_refs: vec![],
                always_load: false,
                intent_keywords: vec!["pr".into(), "ci".into()],
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
                argument_hint: String::new(),
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
                argument_hint: String::new(),
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
    fn lazy_select_includes_intent_matched_skills() {
        let reg = sample_registry();
        let picked = reg.select_for_message("Why is CI failing on PR #42?", true);
        let names: Vec<_> = picked.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"general-agent-tone"));
        assert!(names.contains(&"ci-triage"));
        assert!(!names.contains(&"pr-merge"));
    }

    #[test]
    fn format_catalog_lists_name_and_description() {
        let reg = sample_registry();
        let cat = reg.format_catalog();
        assert!(cat.contains("**ci-triage**"));
        assert!(cat.contains("**pr-merge**"));
        assert!(cat.contains("skill_load"));
    }

    #[test]
    fn intent_select_always_plus_ci() {
        let reg = sample_registry();
        let picked = reg.select_for_message_by_intent("Why is CI failing on PR #42?");
        let names: Vec<_> = picked.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"general-agent-tone"));
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
        assert_eq!(reg.select_for_message("hello", false).len(), 4);
    }

    #[test]
    fn intent_select_issue_tracker() {
        let reg = SkillRegistry::from_skills(vec![SkillSpec {
            name: "issue-tracker".into(),
            description: "issues".into(),
            body: String::new(),
            argument_hint: String::new(),
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
            argument_hint: String::new(),
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
    fn intent_ci_health_penalizes_pr_questions() {
        let skill = super::super::skill::load_skill("skills/ci-health/SKILL.md").unwrap();
        let reg = SkillRegistry::from_skills(vec![skill]);
        let picked = reg.select_for_message_by_intent("Why is CI failing on PR #42?");
        assert!(!picked.iter().any(|s| s.name == "ci-health"));
        let branch = reg.select_for_message_by_intent("How is main branch CI health?");
        assert!(branch.iter().any(|s| s.name == "ci-health"));
    }

    #[test]
    fn merge_skills_adds_unlisted_skills_without_duplicates() {
        // The chat prompt lists github-ops-tone + ci-triage; a directory scan
        // finds those PLUS my-prs (which the prompt doesn't list). The merge
        // should include my-prs without duplicating the two already present.
        let prompt_skills = vec![
            SkillSpec {
                name: "github-ops-tone".into(),
                description: "tone".into(),
                ..Default::default()
            },
            SkillSpec {
                name: "ci-triage".into(),
                description: "ci".into(),
                ..Default::default()
            },
        ];
        let scanned = vec![
            SkillSpec {
                name: "github-ops-tone".into(),
                description: "tone".into(),
                ..Default::default()
            },
            SkillSpec {
                name: "my-prs".into(),
                description: "my prs".into(),
                ..Default::default()
            },
        ];
        let mut merged = prompt_skills;
        merge_skills(&mut merged, scanned);
        let names: Vec<_> = merged.iter().map(|s| s.name.clone()).collect();
        assert!(names.contains(&"github-ops-tone".to_string()));
        assert!(names.contains(&"ci-triage".to_string()));
        assert!(
            names.contains(&"my-prs".to_string()),
            "my-prs must be merged in"
        );
        // No duplicates.
        assert_eq!(
            merged
                .iter()
                .filter(|s| s.name == "github-ops-tone")
                .count(),
            1
        );
    }
}
