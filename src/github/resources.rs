use regex::Regex;
use std::sync::OnceLock;

use crate::error::{CoworkerError, Result};

#[derive(Debug, Clone)]
pub struct ParsedResource {
    pub tool: &'static str,
    pub args: serde_json::Value,
}

pub fn parse_github_resource_uri(uri: &str) -> Option<ParsedResource> {
    let uri = uri.trim();
    if let Some(m) = pr_overview_re().captures(uri) {
        let pr: u32 = m[3].parse().ok()?;
        return Some(ParsedResource {
            tool: "pr_get_overview",
            args: serde_json::json!({
                "repo": format!("{}/{}", &m[1], &m[2]),
                "pr_number": pr,
            }),
        });
    }
    if let Some(m) = pr_blockers_re().captures(uri) {
        let pr: u32 = m[3].parse().ok()?;
        return Some(ParsedResource {
            tool: "pr_get_merge_blockers",
            args: serde_json::json!({
                "repo": format!("{}/{}", &m[1], &m[2]),
                "pr_number": pr,
            }),
        });
    }
    if let Some(m) = pr_ci_re().captures(uri) {
        let pr: u32 = m[3].parse().ok()?;
        return Some(ParsedResource {
            tool: "ci_analyze_pr_failures",
            args: serde_json::json!({
                "repo": format!("{}/{}", &m[1], &m[2]),
                "pr_number": pr,
            }),
        });
    }
    if let Some(m) = pr_ci_snapshot_re().captures(uri) {
        let pr: u32 = m[3].parse().ok()?;
        return Some(ParsedResource {
            tool: "pr_get_ci_snapshot",
            args: serde_json::json!({
                "repo": format!("{}/{}", &m[1], &m[2]),
                "pr_number": pr,
            }),
        });
    }
    if let Some(m) = pr_review_re().captures(uri) {
        let pr: u32 = m[3].parse().ok()?;
        return Some(ParsedResource {
            tool: "pr_get_review_state",
            args: serde_json::json!({
                "repo": format!("{}/{}", &m[1], &m[2]),
                "pr_number": pr,
            }),
        });
    }
    if let Some(m) = branch_health_re().captures(uri) {
        return Some(ParsedResource {
            tool: "ci_branch_health",
            args: serde_json::json!({
                "repo": format!("{}/{}", &m[1], &m[2]),
                "branch": &m[3],
            }),
        });
    }
    None
}

pub async fn read_resource_via_dispatch<F, Fut>(uri: &str, dispatch: F) -> Result<String>
where
    F: FnOnce(&'static str, serde_json::Value) -> Fut,
    Fut: std::future::Future<Output = Result<String>>,
{
    let parsed = parse_github_resource_uri(uri).ok_or_else(|| {
        CoworkerError::Workflow(format!("unsupported resource URI: {uri}"))
    })?;
    dispatch(parsed.tool, parsed.args).await
}

fn pr_overview_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^github://pull/([^/]+)/([^/]+)/(\d+)/overview$").unwrap()
    })
}

fn pr_blockers_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^github://pull/([^/]+)/([^/]+)/(\d+)/blockers$").unwrap()
    })
}

fn pr_ci_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^github://pull/([^/]+)/([^/]+)/(\d+)/ci$").unwrap())
}

fn pr_ci_snapshot_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^github://pull/([^/]+)/([^/]+)/(\d+)/ci-snapshot$").unwrap()
    })
}

fn pr_review_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^github://pull/([^/]+)/([^/]+)/(\d+)/review$").unwrap()
    })
}

fn branch_health_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^github://repo/([^/]+)/([^/]+)/branch/([^/]+)/ci-health$").unwrap()
    })
}
