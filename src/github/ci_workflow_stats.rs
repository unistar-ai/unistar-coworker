use std::collections::HashMap;
use std::time::Duration;

use serde_json::Value;

use super::args::{optional_str, optional_u32, require_str};
use super::ci_common::{self, BranchRun};
use super::exec::GhExec;
use crate::error::Result;

struct WorkflowStats {
    runs: u32,
    failures: u32,
    duration_sum: Duration,
    duration_samples: u32,
    max_duration: Duration,
}

pub async fn ci_workflow_stats(exec: &GhExec, args: &Value) -> Result<String> {
    let repo = require_str(args, "repo")?;
    let mut branch = optional_str(args, "branch").unwrap_or_default();
    if branch.is_empty() {
        branch = ci_common::default_branch(exec, &repo).await?;
    }
    let limit = optional_u32(args, "limit", 30);
    let mut top = optional_u32(args, "top", 10);
    if top == 0 {
        top = 10;
    }
    if top > 20 {
        top = 20;
    }
    let runs = ci_common::list_branch_runs(exec, &repo, &branch, limit).await?;
    Ok(build_workflow_stats_text(&repo, &branch, &runs, top))
}

fn aggregate_workflow_stats(runs: &[BranchRun]) -> HashMap<String, WorkflowStats> {
    let mut out = HashMap::new();
    for r in runs {
        let c = ci_common::run_conclusion(r);
        if ci_common::run_status_in_progress(&c) {
            continue;
        }
        let name = r.workflow_name.trim();
        let name = if name.is_empty() {
            "(unknown)".to_string()
        } else {
            name.to_string()
        };
        let st = out.entry(name).or_insert(WorkflowStats {
            runs: 0,
            failures: 0,
            duration_sum: Duration::ZERO,
            duration_samples: 0,
            max_duration: Duration::ZERO,
        });
        st.runs += 1;
        if ci_common::is_failed_conclusion(&c) {
            st.failures += 1;
        }
        let d = ci_common::run_duration(&r.created_at, &r.updated_at, &c);
        if d > Duration::ZERO {
            st.duration_sum += d;
            st.duration_samples += 1;
            if d > st.max_duration {
                st.max_duration = d;
            }
        }
    }
    out
}

fn build_workflow_stats_text(repo: &str, branch: &str, runs: &[BranchRun], top: u32) -> String {
    let by_wf = aggregate_workflow_stats(runs);
    let mut lines = vec![format!(
        "Workflow stats: {repo}  branch {branch}  ({} runs sampled)",
        runs.len()
    )];
    if by_wf.is_empty() {
        lines.push("No completed workflow runs in sample.".into());
        lines.push("Next: ci_list_runs or widen limit.".into());
        return lines.join("\n");
    }

    let mut rows: Vec<(String, WorkflowStats)> = by_wf.into_iter().collect();
    rows.sort_by(|a, b| {
        b.1.failures
            .cmp(&a.1.failures)
            .then_with(|| b.1.runs.cmp(&a.1.runs))
    });
    rows.truncate(top as usize);

    lines.push("workflow  runs  failures  fail_rate  avg_dur  max_dur".into());
    for (name, st) in &rows {
        let rate = if st.runs > 0 {
            format!("{:.0}%", st.failures as f64 * 100.0 / st.runs as f64)
        } else {
            "-".into()
        };
        let avg = if st.duration_samples > 0 {
            ci_common::format_duration_compact(st.duration_sum / st.duration_samples)
        } else {
            "-".into()
        };
        let max = if st.max_duration > Duration::ZERO {
            ci_common::format_duration_compact(st.max_duration)
        } else {
            "-".into()
        };
        lines.push(format!(
            "{name}  {}  {}  {rate}  {avg}  {max}",
            st.runs, st.failures
        ));
    }
    lines.push(
        "Next: ci_branch_health for streak; ci_get_run_summary on failing workflow.".into(),
    );
    lines.join("\n")
}
