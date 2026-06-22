use serde_json::Value;

use super::args::{require_str, require_u32};
use super::checks;
use super::ci_common;
use super::exec::GhExec;
use crate::error::Result;

pub async fn ci_get_check_url(exec: &GhExec, args: &Value) -> Result<String> {
    let repo = require_str(args, "repo")?;
    let pr_num = require_u32(args, "pr_number")?;
    let rollup = ci_common::pr_status_rollup(exec, &repo, pr_num).await?;

    let mut lines = Vec::new();
    for c in &rollup {
        if c.typename.as_deref() != Some("StatusContext") {
            continue;
        }
        let name = checks::check_display_name(c);
        if name.is_empty() {
            continue;
        }
        let url = checks::check_details_url(c);
        let verdict = checks::check_verdict(c).to_ascii_lowercase();
        if !url.is_empty() {
            lines.push(format!("- {name}: {verdict}  {url}"));
        } else {
            lines.push(format!("- {name}: {verdict}  (no URL in API)"));
        }
    }

    if lines.is_empty() {
        return Ok(format!(
            "No external status checks with URLs on PR #{pr_num} in {repo}.\nUse ci_analyze_pr_failures for GitHub Actions."
        ));
    }

    let mut out = format!(
        "{} external check(s) with URLs on PR #{pr_num}:\n",
        lines.len()
    );
    out.push_str(&lines.join("\n"));
    out.push_str("\n\nDo not call ci_get_failed_logs for these checks.");
    Ok(out)
}
