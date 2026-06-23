//! Session runtime context: workspace + git summary for coding chat.

use std::path::Path;
use std::process::Command;

use crate::error::Result;

#[derive(Debug, Clone)]
pub struct RuntimeContextPlan {
    /// Full body for the TUI context panel.
    pub full_body: String,
    /// Delta (or initial full) body for the LLM; empty when unchanged and no focus.
    pub llm_body: String,
    pub revision: u64,
    pub skip_llm_injection: bool,
    pub new_state: crate::store::ChatRuntimeState,
}

pub struct RuntimeContextInput<'a> {
    pub workspace_path: &'a str,
    pub git_summary: &'a str,
    pub recent_edits: &'a [String],
    pub loaded_skills: Vec<String>,
    pub focus_lines: Vec<String>,
    /// Workspace `AGENTS.md` body (injected once per workspace in session context).
    pub project_instructions: Option<&'a str>,
    pub prev_state: Option<&'a crate::store::ChatRuntimeState>,
}

const AGENTS_MD_MAX_CHARS: usize = 8_000;

/// Load `AGENTS.md` from the chat workspace (project conventions for the model).
pub fn load_workspace_agents_md(workspace: &Path) -> Option<String> {
    let path = workspace.join("AGENTS.md");
    if !path.is_file() {
        return None;
    }
    let text = std::fs::read_to_string(&path).ok()?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(crate::agent::context::truncate_chars(
        trimmed,
        AGENTS_MD_MAX_CHARS,
    ))
}

pub fn build_workspace_git_summary(workspace: &Path) -> String {
    let branch = git_branch(workspace);
    let dirty = git_dirty_count(workspace);
    match (branch, dirty) {
        (Some(b), Some(n)) => format!("(branch: {b}, dirty: {n} files)"),
        (Some(b), None) => format!("(branch: {b})"),
        (None, Some(n)) => format!("(dirty: {n} files)"),
        (None, None) => String::new(),
    }
}

fn git_branch(workspace: &Path) -> Option<String> {
    let out = Command::new("git")
        .args(["-C", &workspace.to_string_lossy(), "branch", "--show-current"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let branch = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if branch.is_empty() {
        None
    } else {
        Some(branch)
    }
}

fn git_dirty_count(workspace: &Path) -> Option<u32> {
    let out = Command::new("git")
        .args(["-C", &workspace.to_string_lossy(), "status", "--short"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let count = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .count() as u32;
    Some(count)
}

pub fn plan_runtime_context(input: RuntimeContextInput<'_>) -> RuntimeContextPlan {
    let current_state = crate::store::ChatRuntimeState {
        revision: 0,
        workspace_path: input.workspace_path.to_string(),
        git_summary: input.git_summary.to_string(),
        recent_edits: input.recent_edits.to_vec(),
        loaded_skills: input.loaded_skills.clone(),
    };

    let full_body = format_full_runtime_body(
        input.workspace_path,
        input.git_summary,
        input.recent_edits,
        &input.loaded_skills,
        &input.focus_lines,
        input.project_instructions,
    );

    let Some(prev) = input
        .prev_state
        .filter(|s| s.revision > 0 || !s.workspace_path.is_empty())
    else {
        let revision = 1;
        let mut state = current_state;
        state.revision = revision;
        let llm_body = full_body.clone();
        let skip = llm_body.trim().is_empty();
        return RuntimeContextPlan {
            full_body,
            llm_body,
            revision,
            skip_llm_injection: skip,
            new_state: state,
        };
    };

    let workspace_changed = prev.workspace_path != input.workspace_path;
    let git_changed = prev.git_summary != input.git_summary;
    let edits_changed = prev.recent_edits != input.recent_edits;
    let skills_changed = prev.loaded_skills != input.loaded_skills;

    let mut delta_parts = Vec::new();

    if workspace_changed {
        delta_parts.push(format!(
            "## Workspace (changed)\n{} {}",
            input.workspace_path, input.git_summary
        ));
    } else if git_changed {
        delta_parts.push(format!("## Workspace git (changed)\n{}", input.git_summary));
    }

    let edits_delta = diff_lines(&prev.recent_edits, input.recent_edits);
    if !edits_delta.is_empty() {
        delta_parts.push(format!("## Recent edits (updated)\n{}", edits_delta.join("\n")));
    }

    if let Some(skill_delta) = diff_skills(&prev.loaded_skills, &input.loaded_skills) {
        delta_parts.push(format!("## Loaded skills (updated)\n{skill_delta}"));
    }

    if !input.focus_lines.is_empty() {
        delta_parts.push(format!(
            "## Message focus\n{}",
            input.focus_lines.join("\n")
        ));
    }

    if delta_parts.is_empty() {
        return RuntimeContextPlan {
            full_body,
            llm_body: String::new(),
            revision: prev.revision,
            skip_llm_injection: true,
            new_state: prev.clone(),
        };
    }

    let revision = if workspace_changed || git_changed || edits_changed || skills_changed {
        prev.revision.saturating_add(1)
    } else {
        prev.revision
    };

    let mut new_state = current_state;
    new_state.revision = revision;

    let llm_header = if workspace_changed || git_changed || edits_changed || skills_changed {
        format!("(runtime context revision {revision})")
    } else {
        "(message focus)".into()
    };

    let llm_body = format!("{llm_header}\n{}", delta_parts.join("\n\n"));

    RuntimeContextPlan {
        full_body,
        llm_body,
        revision,
        skip_llm_injection: false,
        new_state,
    }
}

fn diff_lines(prev: &[String], curr: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for line in curr {
        if !prev.contains(line) {
            out.push(if prev.is_empty() {
                line.clone()
            } else {
                format!("+ {line}")
            });
        }
    }
    for line in prev {
        if !curr.contains(line) {
            out.push(format!("- {line}"));
        }
    }
    out
}

fn diff_skills(prev: &[String], curr: &[String]) -> Option<String> {
    let added: Vec<_> = curr.iter().filter(|s| !prev.contains(s)).collect();
    let removed: Vec<_> = prev.iter().filter(|s| !curr.contains(s)).collect();
    if added.is_empty() && removed.is_empty() {
        return None;
    }
    let mut parts = Vec::new();
    if !added.is_empty() {
        parts.push(format!(
            "+ {}",
            added
                .iter()
                .map(|s| format!("`{s}`"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if !removed.is_empty() {
        parts.push(format!(
            "- {}",
            removed
                .iter()
                .map(|s| format!("`{s}`"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    Some(parts.join("; "))
}

fn format_full_runtime_body(
    workspace_path: &str,
    git_summary: &str,
    recent_edits: &[String],
    loaded_skills: &[String],
    focus_lines: &[String],
    project_instructions: Option<&str>,
) -> String {
    let git_part = if git_summary.trim().is_empty() {
        String::new()
    } else {
        format!(" {git_summary}")
    };
    let mut parts = vec![format!("## Workspace\n{workspace_path}{git_part}")];
    if let Some(body) = project_instructions.filter(|s| !s.trim().is_empty()) {
        parts.push(format!("## Project instructions (AGENTS.md)\n{body}"));
    }
    if recent_edits.is_empty() {
        parts.push("## Recent edits (this session)\n(none yet)".into());
    } else {
        parts.push(format!(
            "## Recent edits (this session)\n{}",
            recent_edits.join("\n")
        ));
    }
    if !loaded_skills.is_empty() {
        parts.push(format!(
            "## Loaded skills\n{}",
            loaded_skills
                .iter()
                .map(|s| format!("`{s}`"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if !focus_lines.is_empty() {
        parts.push(format!("## Message focus\n{}", focus_lines.join("\n")));
    }
    parts.join("\n\n")
}

/// Coding chat message focus — lightweight keyword hints (no store PR snapshots).
pub async fn build_message_focus_lines(
    _store: &dyn crate::store::Store,
    user_message: &str,
    _configured_repos: &[String],
) -> Result<Vec<String>> {
    use crate::agent::chat_discovery::{extract_github_pr_link, message_looks_like_pr_task};

    let lower = user_message.to_ascii_lowercase();
    let mut lines = Vec::new();
    if let Some((repo, pr)) = extract_github_pr_link(user_message) {
        lines.push(format!(
            "GitHub PR linked: {repo}#{pr} — if `pr-review` is not loaded yet, skill_load it; then pr_get_overview(repo=\"{repo}\", pr_number={pr})"
        ));
    } else if message_looks_like_pr_task(user_message) {
        lines.push(
            "PR task — skill_load `pr-review` if not already loaded, then harness pr_get_* tools (not web_browser)".into(),
        );
    }
    if lower.contains("test") || lower.contains("cargo") || lower.contains("npm") {
        lines.push("hint: run tests/build with bash_run after edits".into());
    }
    if lower.contains("fix") || lower.contains("bug") || lower.contains("error") {
        lines.push("hint: read error output and grep before editing".into());
    }
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_turn_injects_full_context() {
        let plan = plan_runtime_context(RuntimeContextInput {
            workspace_path: "/proj",
            git_summary: "(branch: main, dirty: 2 files)",
            recent_edits: &["src/a.rs (+3 -1)".into()],
            loaded_skills: vec!["code-edit".into()],
            focus_lines: vec!["hint: grep".into()],
            project_instructions: None,
            prev_state: None,
        });
        assert!(!plan.skip_llm_injection);
        assert_eq!(plan.revision, 1);
        assert!(plan.full_body.contains("Workspace"));
        assert!(plan.full_body.contains("Recent edits"));
        assert!(plan.llm_body.contains("hint: grep"));
    }

    #[test]
    fn unchanged_workspace_skips_llm_injection() {
        let prev = crate::store::ChatRuntimeState {
            revision: 2,
            workspace_path: "/proj".into(),
            git_summary: "(branch: main)".into(),
            recent_edits: vec!["src/a.rs (+1)".into()],
            loaded_skills: vec!["code-edit".into()],
        };
        let plan = plan_runtime_context(RuntimeContextInput {
            workspace_path: "/proj",
            git_summary: "(branch: main)",
            recent_edits: &prev.recent_edits,
            loaded_skills: prev.loaded_skills.clone(),
            focus_lines: vec![],
            project_instructions: None,
            prev_state: Some(&prev),
        });
        assert!(plan.skip_llm_injection);
        assert_eq!(plan.revision, 2);
    }

    #[test]
    fn project_instructions_in_full_body() {
        let plan = plan_runtime_context(RuntimeContextInput {
            workspace_path: "/proj",
            git_summary: "",
            recent_edits: &[],
            loaded_skills: vec![],
            focus_lines: vec![],
            project_instructions: Some("Run `cargo test` before finishing."),
            prev_state: None,
        });
        assert!(plan.full_body.contains("Project instructions"));
        assert!(plan.full_body.contains("cargo test"));
    }

    #[test]
    fn edits_delta_bumps_revision() {
        let prev = crate::store::ChatRuntimeState {
            revision: 1,
            workspace_path: "/proj".into(),
            git_summary: "(branch: main)".into(),
            recent_edits: vec!["src/a.rs (+1)".into()],
            loaded_skills: vec![],
        };
        let plan = plan_runtime_context(RuntimeContextInput {
            workspace_path: "/proj",
            git_summary: "(branch: main)",
            recent_edits: &["src/a.rs (+1)".into(), "src/b.rs (+2)".into()],
            loaded_skills: vec![],
            focus_lines: vec![],
            project_instructions: None,
            prev_state: Some(&prev),
        });
        assert!(!plan.skip_llm_injection);
        assert_eq!(plan.revision, 2);
        assert!(plan.llm_body.contains("+ src/b.rs"));
    }
}
