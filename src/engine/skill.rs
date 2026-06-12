use std::path::Path;

use crate::error::{CoworkerError, Result};

#[derive(Debug, Clone)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub body: String,
}

pub fn load_skill(path: impl AsRef<Path>) -> Result<Skill> {
    let raw = std::fs::read_to_string(path.as_ref()).map_err(|e| {
        CoworkerError::Workflow(format!("read skill {}: {e}", path.as_ref().display()))
    })?;
    parse_skill(&raw)
}

pub fn parse_skill(raw: &str) -> Result<Skill> {
    let trimmed = raw.trim_start();
    if !trimmed.starts_with("---") {
        return Ok(Skill {
            name: String::new(),
            description: String::new(),
            body: raw.to_string(),
        });
    }

    let rest = trimmed.strip_prefix("---").unwrap_or(trimmed).trim_start();
    let Some((front, body)) = rest.split_once("\n---") else {
        return Ok(Skill {
            name: String::new(),
            description: String::new(),
            body: raw.to_string(),
        });
    };

    let mut name = String::new();
    let mut description = String::new();
    for line in front.lines() {
        let line = line.trim();
        if let Some(v) = line.strip_prefix("name:") {
            name = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("description:") {
            description = v.trim().to_string();
        }
    }

    Ok(Skill {
        name,
        description,
        body: body.trim_start_matches('\n').trim().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_frontmatter() {
        let raw = "---\nname: daily-work\ndescription: triage\n---\n\n# Body\n";
        let s = parse_skill(raw).unwrap();
        assert_eq!(s.name, "daily-work");
        assert_eq!(s.description, "triage");
        assert!(s.body.contains("# Body"));
    }
}
