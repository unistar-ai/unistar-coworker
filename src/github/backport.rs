use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;

use super::args::{optional_u32, require_str, require_u32};
use super::error::{format_tool_error, format_tool_ok, ErrCode};
use super::exec::{GhExec, GitExec};
use crate::error::{CoworkerError, Result};

const SERVER_NAME: &str = "unistar-coworker";

pub async fn pr_create_backport(exec: &GhExec, args: &Value) -> Result<String> {
    let git = GitExec::from_gh(exec);
    let repo = require_str(args, "repo")?;
    let pr_num = require_u32(args, "pr_number")?;
    let target_branch = require_str(args, "target_branch")?;
    let pr_s = pr_num.to_string();

    let gh_args = [
        "pr",
        "view",
        &pr_s,
        "-R",
        &repo,
        "--json",
        "mergeCommit,title,body",
    ];
    let res = exec.run_retry(&gh_args).await;
    let stdout = GhExec::into_result(res, "failed to fetch PR details")?;
    #[derive(Deserialize)]
    struct PrBackportInfo {
        #[serde(rename = "mergeCommit")]
        merge_commit: Option<MergeCommit>,
        title: String,
        body: String,
    }
    #[derive(Deserialize)]
    struct MergeCommit {
        oid: String,
    }
    let info: PrBackportInfo = serde_json::from_str(&stdout)
        .map_err(|e| CoworkerError::Other(anyhow::anyhow!("failed to parse PR details: {e}")))?;
    let merge_commit = info
        .merge_commit
        .map(|m| m.oid)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            CoworkerError::Other(anyhow::anyhow!(
                "PR #{pr_num} does not have a merge commit (is it merged?)."
            ))
        })?;

    let who = gh_current_user(exec).await;
    let branch_name = format!("backport-{pr_num}-to-{}", sanitize_ref(&target_branch));

    let work_path = std::env::temp_dir().join(format!("unistar-backport-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&work_path).map_err(|e| {
        CoworkerError::Other(anyhow::anyhow!("failed to create temporary workspace: {e}"))
    })?;

    let clone_args = [
        "repo",
        "clone",
        &repo,
        work_path.to_str().unwrap_or("."),
        "--",
        "--depth",
        "1",
        "--branch",
        &target_branch,
    ];
    let clone_res = exec.run_retry(&clone_args).await;
    if clone_res.err.is_some() {
        return Err(clone_res.wrap(&format!(
            "failed to clone {repo} at branch {target_branch:?} (does the branch exist?)"
        )));
    }

    let checkout_res = git
        .run(
            Some(work_path.to_str().unwrap()),
            &["checkout", "-B", &branch_name],
        )
        .await;
    if checkout_res.err.is_some() {
        return Err(checkout_res.wrap("failed to create backport branch"));
    }

    let fetch_res = git
        .run(
            Some(work_path.to_str().unwrap()),
            &["fetch", "--depth", "2", "origin", &merge_commit],
        )
        .await;
    if fetch_res.err.is_some() {
        return Err(fetch_res.wrap("failed to fetch the PR merge commit"));
    }

    let mut cp_args = vec!["cherry-pick", "-x"];
    if is_merge_commit(&git, &work_path, &merge_commit).await {
        cp_args.push("-m");
        cp_args.push("1");
    }
    cp_args.push(&merge_commit);
    let cp_res = git.run(Some(work_path.to_str().unwrap()), &cp_args).await;
    if cp_res.err.is_some() {
        let work_str = work_path.display().to_string();
        return Err(CoworkerError::Other(anyhow::anyhow!(
            "Cherry-pick of {} onto {} failed (likely a conflict). \
             The cherry-pick is left in progress on branch {:?} in the temporary workspace {}.\n\n{}\n\n\
             To finish manually:\n  1. cd {}\n  2. resolve conflicts, then: git add -A && git cherry-pick --continue\n  \
             3. git push -u origin {}\n  4. gh pr create -R {} --base {} --head {} --title \"[backport -> {}] {}\" --body \"Automated backport of #{}\"\n  \
             5. remove the workspace: rm -rf {}\n\nTo give up instead: rm -rf {}",
            short_sha(&merge_commit),
            target_branch,
            branch_name,
            work_str,
            cp_res.combined(),
            work_str,
            branch_name,
            repo,
            target_branch,
            branch_name,
            target_branch,
            info.title,
            pr_num,
            work_str,
            work_str
        )));
    }

    let push_res = git
        .run(
            Some(work_path.to_str().unwrap()),
            &["push", "-u", "origin", &branch_name],
        )
        .await;
    if push_res.err.is_some() {
        return Err(push_res.wrap("failed to push backport branch"));
    }

    let title = format!("[backport -> {target_branch}] {}", info.title);
    let body = backport_body(&target_branch, &who, &info.body);
    let create_res = git
        .run_gh_in_dir(
            exec,
            work_path.to_str().unwrap(),
            &[
                "pr",
                "create",
                "-R",
                &repo,
                "--base",
                &target_branch,
                "--head",
                &branch_name,
                "--title",
                &title,
                "--body",
                &body,
            ],
        )
        .await;
    if create_res.err.is_some() {
        return Err(
            create_res.wrap("cherry-pick succeeded and branch pushed, but failed to create PR")
        );
    }

    Ok(format_tool_ok(&format!(
        "Backport PR opened: {}",
        create_res.stdout.trim()
    )))
}

pub async fn backport_get_conflict_files(exec: &GhExec, args: &Value) -> Result<String> {
    let git = GitExec::from_gh(exec);
    let work_dir = require_str(args, "workspace_path")?;
    validate_backport_workspace(&work_dir)?;
    if !Path::new(&work_dir).exists() {
        return Err(CoworkerError::Other(anyhow::anyhow!(format_tool_error(
            ErrCode::NotFound,
            &work_dir,
            "workspace may have been removed — rerun pr_create_backport",
        ))));
    }

    let res = git
        .run(Some(&work_dir), &["diff", "--name-only", "--diff-filter=U"])
        .await;
    if res.err.is_some() {
        return Err(res.wrap("failed to list conflict files"));
    }
    let files: Vec<&str> = res.stdout.split_whitespace().collect();
    if files.is_empty() {
        return Ok(
            "No unmerged conflict files (cherry-pick may not be in conflict state).\n\
             hint: run from the workspace left by pr_create_backport"
                .into(),
        );
    }

    let mut lines = vec![format!("{} conflict file(s) in {work_dir}:", files.len())];
    for f in &files {
        lines.push(format!("- {f}"));
    }
    if let Some(first) = files.first() {
        let diff_res = git.run(Some(&work_dir), &["diff", "--", first]).await;
        if diff_res.err.is_none() && !diff_res.stdout.trim().is_empty() {
            lines.push("\nConflict snippet (first file, capped):".into());
            lines.push(clip_for_log(&diff_res.stdout, 1500));
        }
    }
    lines.push("Next: resolve in workspace, git add -A && git cherry-pick --continue".into());
    Ok(lines.join("\n"))
}

pub async fn backport_suggest_resolution(exec: &GhExec, args: &Value) -> Result<String> {
    let git = GitExec::from_gh(exec);
    let work_dir = require_str(args, "workspace_path")?;
    validate_backport_workspace(&work_dir)?;
    if !Path::new(&work_dir).exists() {
        return Err(CoworkerError::Other(anyhow::anyhow!(format_tool_error(
            ErrCode::NotFound,
            &work_dir,
            "workspace may have been removed — rerun pr_create_backport",
        ))));
    }

    let mut max_files = optional_u32(args, "max_files", 3);
    if max_files == 0 {
        max_files = 3;
    }
    if max_files > 10 {
        max_files = 10;
    }

    let res = git
        .run(Some(&work_dir), &["diff", "--name-only", "--diff-filter=U"])
        .await;
    if res.err.is_some() {
        return Err(res.wrap("failed to list conflict files"));
    }
    let files: Vec<&str> = res.stdout.split_whitespace().collect();
    if files.is_empty() {
        return Ok(
            "No unmerged conflict files — cherry-pick may not be in conflict state.\n\
             Next: backport_get_conflict_files to verify."
                .into(),
        );
    }

    let mut lines = vec![format!(
        "Resolution hints for {} conflict file(s) in {work_dir}:",
        files.len()
    )];
    let analyze = files.len().min(max_files as usize);
    for f in files.iter().take(analyze) {
        let path = PathBuf::from(&work_dir).join(f);
        match std::fs::read_to_string(&path) {
            Ok(content) => {
                let (ours, theirs, hint) = analyze_conflict_markers(&content);
                lines.push(format!(
                    "\n{f}:\n  ours:{ours} lines  theirs:{theirs} lines\n  hint: {hint}"
                ));
            }
            Err(_) => {
                lines.push(format!("\n{f}: (could not read file)"));
            }
        }
    }
    if files.len() > analyze {
        lines.push(format!(
            "\n({} more file(s) — raise max_files or use backport_get_conflict_files)",
            files.len() - analyze
        ));
    }
    lines.push("Next: resolve markers, git add -A && git cherry-pick --continue".into());
    Ok(lines.join("\n"))
}

fn validate_backport_workspace(path: &str) -> Result<()> {
    let base = Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    if !base.starts_with("unistar-backport-") {
        return Err(CoworkerError::Other(anyhow::anyhow!(format_tool_error(
            ErrCode::Validation,
            "path is not a unistar backport workspace",
            "use the exact path from pr_create_backport conflict output",
        ))));
    }
    Ok(())
}

async fn gh_current_user(exec: &GhExec) -> String {
    let res = exec.run_retry(&["api", "user", "-q", ".login"]).await;
    let login = res.stdout.trim();
    if res.err.is_some() || login.is_empty() {
        "unknown".into()
    } else {
        login.to_string()
    }
}

fn backport_body(target_branch: &str, who: &str, original_body: &str) -> String {
    format!(
        "Automated backport to `{target_branch}`, triggered by @{who}, using MCP `{SERVER_NAME}`\n\n\
         ## Original Description\n{original_body}"
    )
}

async fn is_merge_commit(git: &GitExec, dir: &Path, commit: &str) -> bool {
    let dir_s = dir.to_str().unwrap_or(".");
    let res = git
        .run(Some(dir_s), &["rev-list", "--parents", "-n", "1", commit])
        .await;
    res.stdout.split_whitespace().count() > 2
}

fn sanitize_ref(s: &str) -> String {
    s.replace(['/', ' '], "-")
}

fn short_sha(sha: &str) -> String {
    if sha.len() > 7 {
        sha[..7].to_string()
    } else {
        sha.to_string()
    }
}

fn clip_for_log(s: &str, limit: usize) -> String {
    if s.len() <= limit {
        s.to_string()
    } else {
        format!("{}…[truncated]", &s[..limit])
    }
}

fn analyze_conflict_markers(content: &str) -> (usize, usize, &'static str) {
    let mut in_ours = false;
    let mut in_theirs = false;
    let mut ours = 0usize;
    let mut theirs = 0usize;
    for line in content.lines() {
        let trim = line.trim();
        if trim.starts_with("<<<<<<<") {
            in_ours = true;
            in_theirs = false;
            continue;
        }
        if trim == "=======" {
            in_ours = false;
            in_theirs = true;
            continue;
        }
        if trim.starts_with(">>>>>>>") {
            in_theirs = false;
            continue;
        }
        if in_ours {
            ours += 1;
        } else if in_theirs {
            theirs += 1;
        }
    }
    let hint = match (ours, theirs) {
        (0, t) if t > 0 => "only target-branch content — consider accepting incoming (theirs)",
        (o, 0) if o > 0 => "only cherry-pick content — consider keeping ours",
        (o, t) if o > 0 && t > 0 => {
            "both sides edited — manual merge; compare semantics before choosing"
        }
        _ => "markers present but no distinct hunks — inspect file manually",
    };
    (ours, theirs, hint)
}
