//! Interim GitHub discovery until unistar-mcp adds `pr_list_merged`.
//! Read-only `gh pr list` — same binary unistar-mcp uses internally.

use serde::Deserialize;
use tokio::process::Command;

use crate::error::{CoworkerError, Result};

#[derive(Debug, Clone)]
pub struct MergedPr {
    pub number: u32,
    pub title: String,
}

#[derive(Debug, Deserialize)]
struct GhPullRequest {
    number: u32,
    title: String,
    state: String,
    labels: Vec<GhLabel>,
}

#[derive(Debug, Deserialize)]
struct GhLabel {
    name: String,
}

/// List recently merged PRs that carry `label` (via `gh pr list --state merged`).
pub async fn list_merged_prs_labeled(repo: &str, label: &str, limit: u32) -> Result<Vec<MergedPr>> {
    let output = Command::new("gh")
        .args([
            "pr",
            "list",
            "-R",
            repo,
            "--state",
            "merged",
            "--label",
            label,
            "--limit",
            &limit.to_string(),
            "--json",
            "number,title,state,labels",
        ])
        .output()
        .await
        .map_err(|e| CoworkerError::Workflow(format!("gh not available: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(CoworkerError::Workflow(format!(
            "gh pr list failed: {stderr}"
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let prs: Vec<GhPullRequest> = serde_json::from_str(&stdout)
        .map_err(|e| CoworkerError::Workflow(format!("parse gh pr list: {e}")))?;

    Ok(prs
        .into_iter()
        .filter(|p| p.state.eq_ignore_ascii_case("MERGED"))
        .filter(|p| p.labels.iter().any(|l| l.name == label))
        .map(|p| MergedPr {
            number: p.number,
            title: p.title,
        })
        .collect())
}
