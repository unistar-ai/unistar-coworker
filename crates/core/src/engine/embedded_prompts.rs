//! Prompts compiled into the binary at build time (`include_str!` on `prompts/*.md`).

use std::path::Path;

/// Default chat system prompt (`prompts/chat.md`).
pub const CHAT_MD: &str = include_str!("../../../../prompts/chat.md");

pub const CHAT_PATH: &str = "prompts/chat.md";
pub const LEGACY_AGENT_PATH: &str = "agents/chat/AGENT.md";

/// True when `chat.prompt` should resolve to the embedded chat spec (not read from cwd).
pub fn is_bundled_chat_prompt(path: &Path) -> bool {
    if path.as_os_str().is_empty() {
        return true;
    }
    let normalized = path.to_string_lossy().replace('\\', "/");
    normalized == CHAT_PATH
        || normalized.ends_with(&format!("/{CHAT_PATH}"))
        || normalized == LEGACY_AGENT_PATH
        || normalized.ends_with(&format!("/{LEGACY_AGENT_PATH}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::engine::skill::parse_markdown_spec;

    #[test]
    fn embedded_chat_prompt_parses() {
        let spec = parse_markdown_spec(CHAT_MD).unwrap();
        assert_eq!(spec.name, "chat");
        // skills: frontmatter removed — skills are now auto-discovered from the
        // skills/ directory, not listed in the prompt.
        assert!(spec.skill_refs.is_empty());
    }

    #[test]
    fn bundled_path_detection() {
        assert!(is_bundled_chat_prompt(Path::new("")));
        assert!(is_bundled_chat_prompt(Path::new("prompts/chat.md")));
        assert!(is_bundled_chat_prompt(Path::new("agents/chat/AGENT.md")));
        assert!(!is_bundled_chat_prompt(Path::new("prompts/custom-chat.md")));
    }
}
