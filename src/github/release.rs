use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;

use super::args::{optional_str, optional_u32, require_str};
use super::ci_common::PrMergedRow;
use super::exec::GhExec;
use crate::error::{CoworkerError, Result};

#[derive(Debug, Deserialize)]
struct GitTagRow {
    name: String,
    commit: TagCommit,
}

#[derive(Debug, Deserialize)]
struct TagCommit {
    sha: String,
}

pub async fn release_list_tags(exec: &GhExec, args: &Value) -> Result<String> {
    let repo = require_str(args, "repo")?;
    let mut limit = optional_u32(args, "limit", 20);
    if limit == 0 {
        limit = 20;
    }
    if limit > 50 {
        limit = 50;
    }
    let tags = list_repo_tags(exec, &repo, limit).await?;
    if tags.is_empty() {
        return Ok(format!("No tags found for {repo}."));
    }
    let mut lines = vec![format!("{} tag(s) for {repo} (newest first):", tags.len())];
    for t in &tags {
        let sha = t.commit.sha.trim();
        if sha.len() > 7 {
            lines.push(format!("{}  {}", t.name, &sha[..7]));
        } else if !sha.is_empty() {
            lines.push(format!("{}  {sha}", t.name));
        } else {
            lines.push(t.name.clone());
        }
    }
    lines.push("Next: release_notes_draft with since_tag=<previous release>.".into());
    Ok(lines.join("\n"))
}

pub async fn release_notes_draft(exec: &GhExec, args: &Value) -> Result<String> {
    let repo = require_str(args, "repo")?;
    let mut since_tag = optional_str(args, "since_tag").unwrap_or_default();
    let mut limit = optional_u32(args, "limit", 30);
    if limit == 0 {
        limit = 30;
    }
    if limit > 50 {
        limit = 50;
    }

    if since_tag.is_empty() {
        let tags = list_repo_tags(exec, &repo, 1).await?;
        if tags.is_empty() {
            return Err(CoworkerError::Other(anyhow::anyhow!(
                "no tags in repository; pass since_tag or create a tag first"
            )));
        }
        since_tag = tags[0].name.clone();
    }

    let since_date = tag_commit_date(exec, &repo, &since_tag).await?;
    format_release_notes_bullets(exec, &repo, &since_tag, &since_date, limit).await
}

async fn list_repo_tags(exec: &GhExec, repo: &str, limit: u32) -> Result<Vec<GitTagRow>> {
    let path = format!("repos/{repo}/tags?per_page={limit}");
    let gh_args = ["api", &path];
    let res = exec.run_retry(&gh_args).await;
    let stdout = GhExec::into_result(res, "failed to list tags")?;
    serde_json::from_str(&stdout).map_err(|e| {
        CoworkerError::Other(anyhow::anyhow!("failed to parse tag list: {e}"))
    })
}

async fn tag_commit_date(exec: &GhExec, repo: &str, tag: &str) -> Result<String> {
    let release_path = format!("repos/{repo}/releases/tags/{tag}");
    let release_args = ["api", &release_path];
    let res = exec.run_retry(&release_args).await;
    if res.err.is_none() {
        #[derive(Deserialize)]
        struct ReleaseMeta {
            #[serde(rename = "published_at")]
            published_at: Option<String>,
            #[serde(rename = "created_at")]
            created_at: Option<String>,
        }
        if let Ok(rel) = serde_json::from_str::<ReleaseMeta>(&res.stdout) {
            let ts = rel
                .published_at
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .or_else(|| {
                    rel.created_at
                        .as_deref()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                });
            if let Some(d) = ts.and_then(iso_date_prefix) {
                return Ok(d);
            }
        }
    }

    let ref_path = format!("repos/{repo}/git/ref/tags/{tag}");
    let ref_args = ["api", &ref_path];
    let res = exec.run_retry(&ref_args).await;
    let stdout = GhExec::into_result(res, &format!("failed to resolve tag {tag:?}"))?;
    #[derive(Deserialize)]
    struct TagRef {
        object: TagRefObject,
    }
    #[derive(Deserialize)]
    struct TagRefObject {
        sha: String,
        #[serde(rename = "type")]
        object_type: String,
    }
    let tag_ref: TagRef = serde_json::from_str(&stdout).map_err(|e| {
        CoworkerError::Other(anyhow::anyhow!("failed to parse tag ref: {e}"))
    })?;
    let mut sha = tag_ref.object.sha.trim().to_string();
    if sha.is_empty() {
        return Err(CoworkerError::Other(anyhow::anyhow!(
            "empty commit for tag {tag:?}"
        )));
    }

    if tag_ref.object.object_type.eq_ignore_ascii_case("tag") {
        let tag_obj_path = format!("repos/{repo}/git/tags/{sha}");
        let tag_obj_args = ["api", &tag_obj_path];
        let tag_res = exec.run_retry(&tag_obj_args).await;
        if tag_res.err.is_none() {
            #[derive(Deserialize)]
            struct TagObject {
                object: InnerSha,
            }
            #[derive(Deserialize)]
            struct InnerSha {
                sha: String,
            }
            if let Ok(tag_obj) = serde_json::from_str::<TagObject>(&tag_res.stdout) {
                let inner = tag_obj.object.sha.trim();
                if !inner.is_empty() {
                    sha = inner.to_string();
                }
            }
        }
    }

    let commit_path = format!("repos/{repo}/commits/{sha}");
    let commit_args = ["api", &commit_path, "-q", ".commit.committer.date"];
    let commit_res = exec.run_retry(&commit_args).await;
    let date_raw =
        GhExec::into_result(commit_res, &format!("failed to resolve commit date for tag {tag:?}"))?;
    iso_date_prefix(date_raw.trim()).ok_or_else(|| {
        CoworkerError::Other(anyhow::anyhow!("empty commit date for tag {tag:?}"))
    })
}

fn iso_date_prefix(ts: &str) -> Option<String> {
    if ts.len() >= 10 && ts.as_bytes().get(4) == Some(&b'-') && ts.as_bytes().get(7) == Some(&b'-')
    {
        return Some(ts[..10].to_string());
    }
    DateTime::parse_from_rfc3339(ts)
        .ok()
        .map(|d| d.with_timezone(&Utc).format("%Y-%m-%d").to_string())
}

async fn format_release_notes_bullets(
    exec: &GhExec,
    repo: &str,
    since_tag: &str,
    since_date: &str,
    limit: u32,
) -> Result<String> {
    let search = format!("merged:>={since_date}");
    let limit_s = limit.to_string();
    let gh_args = [
        "pr",
        "list",
        "-R",
        repo,
        "--state",
        "merged",
        "--limit",
        &limit_s,
        "--json",
        "number,title,author,mergedAt",
        "--search",
        &search,
    ];
    let res = exec.run_retry(&gh_args).await;
    let stdout = GhExec::into_result(res, "failed to list merged PRs for release notes")?;
    let prs: Vec<PrMergedRow> = serde_json::from_str(&stdout).map_err(|e| {
        CoworkerError::Other(anyhow::anyhow!("failed to parse merged PR list: {e}"))
    })?;

    let mut lines = vec![format!(
        "Release notes draft for {repo} since tag {since_tag} ({since_date}):"
    )];
    if prs.is_empty() {
        lines.push("(no merged PRs since tag date)".into());
    } else {
        for pr in &prs {
            lines.push(format!(
                "- #{} {} (@{})",
                pr.number, pr.title, pr.author.login
            ));
        }
    }
    lines.push("Next: edit bullets and publish the release or notify_post_slack.".into());
    Ok(lines.join("\n"))
}
