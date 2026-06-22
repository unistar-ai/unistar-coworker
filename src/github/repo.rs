use serde::Deserialize;
use serde_json::Value;

use super::args::{optional_u32, require_str};
use super::exec::GhExec;
use crate::error::{CoworkerError, Result};

const DEFAULT_LABEL_LIMIT: u32 = 20;

#[derive(Debug, Deserialize)]
struct RepoInfo {
    #[serde(default)]
    description: String,
    #[serde(default, rename = "isPrivate")]
    is_private: bool,
    owner: RepoOwner,
    #[serde(default, rename = "defaultBranchRef")]
    default_branch_ref: Option<BranchRef>,
    #[serde(default, rename = "primaryLanguage")]
    primary_language: Option<Lang>,
    #[serde(default, rename = "licenseInfo")]
    license_info: Option<License>,
    #[serde(default, rename = "repositoryTopics")]
    repository_topics: Vec<Topic>,
}

#[derive(Debug, Deserialize)]
struct RepoOwner {
    login: String,
}

#[derive(Debug, Deserialize)]
struct BranchRef {
    name: String,
}

#[derive(Debug, Deserialize)]
struct Lang {
    name: String,
}

#[derive(Debug, Deserialize)]
struct License {
    name: String,
}

#[derive(Debug, Deserialize)]
struct Topic {
    name: String,
}

pub async fn repo_get_info(exec: &GhExec, args: &Value) -> Result<String> {
    let repo = require_str(args, "repo")?;
    let label_limit = optional_u32(args, "label_limit", DEFAULT_LABEL_LIMIT).min(50);
    let gh_args = [
        "repo",
        "view",
        "-R",
        &repo,
        "--json",
        "name,description,isPrivate,owner,defaultBranchRef,primaryLanguage,licenseInfo,repositoryTopics",
    ];
    let res = exec.run_retry(&gh_args).await;
    let stdout = GhExec::into_result(res, "failed to load repo info")?;
    let info: RepoInfo = serde_json::from_str(&stdout).map_err(|e| {
        CoworkerError::Other(anyhow::anyhow!(super::error::format_tool_error(
            super::error::ErrCode::Generic,
            &format!("failed to parse repo info: {e}"),
            "retry",
        )))
    })?;

    let label_args = [
        "api",
        &format!("repos/{repo}/labels"),
        "--paginate",
        "--jq",
        &format!(".[0:{label_limit}] | .[] | .name"),
    ];
    let labels_res = exec.run_retry(&label_args).await;
    let labels: Vec<String> = if labels_res.err.is_none() {
        labels_res
            .stdout
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect()
    } else {
        vec![]
    };

    let default_branch = info
        .default_branch_ref
        .as_ref()
        .map(|b| b.name.as_str())
        .unwrap_or("?");
    let lang = info
        .primary_language
        .as_ref()
        .map(|l| l.name.as_str())
        .unwrap_or("none");
    let license = info
        .license_info
        .as_ref()
        .map(|l| l.name.as_str())
        .unwrap_or("none");
    let topics: Vec<_> = info.repository_topics.iter().map(|t| t.name.as_str()).collect();
    let visibility = if info.is_private { "private" } else { "public" };

    let mut out = format!(
        "Repository: {repo}\nOwner: @{}\nDefault branch: {default_branch}\nVisibility: {visibility}\nLanguage: {lang}\nLicense: {license}",
        info.owner.login
    );
    if !info.description.is_empty() {
        out.push_str(&format!("\nDescription: {}", info.description));
    }
    if !topics.is_empty() {
        out.push_str(&format!("\nTopics: {}", topics.join(", ")));
    }
    if labels.is_empty() {
        out.push_str("\nLabels: (unavailable)");
    } else {
        out.push_str(&format!("\nLabels (up to {label_limit}): {}", labels.join(", ")));
    }
    Ok(out)
}
