//! Chat tool catalog: JSON schemas, contract hints, and fuzzy name suggestions.

use std::borrow::Cow;
use std::collections::HashSet;

use crate::agent::bash_tool::{self, BASH_RUN_TOOL};
use crate::agent::file_tools;
use crate::agent::harness_errors::{
    bash_exit_failure_envelope, bash_validation_envelope, classify_github_error_code,
    file_tool_failure_envelope, generic_tool_failure_envelope, ErrorEnvelope,
};
use crate::agent::harness_tools;
use crate::config::ChatToolMode;

use serde_json::Value;

#[derive(Debug, Clone, Copy)]
pub struct ToolSpec {
    pub name: &'static str,
    pub blurb: &'static str,
    pub required: &'static [&'static str],
    pub optional: &'static [&'static str],
}

const META_TOOLS: &[&str] = &[
    "tool_list",
    "tool_list_category",
    "tool_search",
    "tool_describe",
    "tool_call",
    "resource_read",
    "skill_load",
];

/// Minimal native schemas at chat cold start (lazy/auto). Everything else warms on demand.
const PRELOAD_NATIVE_TOOLS: &[&str] = &[
    "skill_load",
    "tool_search",
    "tool_call",
    "read_file",
    "grep",
    "glob",
    "bash_run",
    "python_run",
    "edit_file",
    "write_file",
    "web_fetch",
];
const MUTATING_TOOLS: &[&str] = &[
    "ci_rerun_workflow",
    "pr_create_backport",
    "pr_post_comment",
    "issue_add_label",
    "notify_post_slack",
];

/// Read-only + meta + mutating tools the chat harness may route.
const TOOLS: &[ToolSpec] = &[
    ToolSpec {
        name: "pr_get_overview",
        blurb: "PR snapshot (status, CI, file counts)",
        required: &["repo", "pr_number"],
        optional: &[],
    },
    ToolSpec {
        name: "pr_list_changed_files",
        blurb: "Changed files with +/- line counts",
        required: &["repo", "pr_number"],
        optional: &[],
    },
    ToolSpec {
        name: "pr_get_diff",
        blurb: "Unified diff (full or single path)",
        required: &["repo", "pr_number"],
        optional: &["path", "max_bytes"],
    },
    ToolSpec {
        name: "repo_get_info",
        blurb: "Repo metadata (default branch, labels, topics)",
        required: &["repo"],
        optional: &["label_limit"],
    },
    ToolSpec {
        name: "pr_list_open",
        blurb: "Open PRs (newest first)",
        required: &["repo"],
        optional: &["author", "limit"],
    },
    ToolSpec {
        name: "pr_list_waiting_review",
        blurb: "CI-green PRs waiting for review",
        required: &["repo"],
        optional: &["limit"],
    },
    ToolSpec {
        name: "pr_get_merge_blockers",
        blurb: "Why a PR cannot merge",
        required: &["repo", "pr_number"],
        optional: &[],
    },
    ToolSpec {
        name: "pr_list_merge_ready",
        blurb: "Merge-ready PRs (CI green, approved)",
        required: &["repo"],
        optional: &["limit"],
    },
    ToolSpec {
        name: "pr_list_merge_blocked",
        blurb: "CI green but blocked PRs",
        required: &["repo"],
        optional: &["limit"],
    },
    ToolSpec {
        name: "pr_draft_ci_comment",
        blurb: "Draft CI failure PR comment",
        required: &["repo", "pr_number", "run_id"],
        optional: &[],
    },
    ToolSpec {
        name: "pr_list_large",
        blurb: "Mega-PR filter by file/line thresholds",
        required: &["repo"],
        optional: &["min_files", "min_lines", "limit"],
    },
    ToolSpec {
        name: "pr_get_review_state",
        blurb: "Reviewers and inline comment summary",
        required: &["repo", "pr_number"],
        optional: &[],
    },
    ToolSpec {
        name: "pr_get_review_routing",
        blurb: "CODEOWNERS-based review routing",
        required: &["repo", "pr_number"],
        optional: &[],
    },
    ToolSpec {
        name: "pr_diff_risk_scan",
        blurb: "Heuristic diff risk flags",
        required: &["repo", "pr_number"],
        optional: &[],
    },
    ToolSpec {
        name: "pr_get_status",
        blurb: "Compact mergeability snapshot",
        required: &["repo", "pr_number"],
        optional: &[],
    },
    ToolSpec {
        name: "pr_get_status_batch",
        blurb: "Batch CI/review for comma-separated PR numbers",
        required: &["repo", "pr_numbers"],
        optional: &[],
    },
    ToolSpec {
        name: "pr_get_overview_batch",
        blurb: "Lightweight multi-PR overview (GraphQL, max 5)",
        required: &["repo", "pr_numbers"],
        optional: &[],
    },
    ToolSpec {
        name: "pr_list_merged",
        blurb: "Recently merged PRs",
        required: &["repo"],
        optional: &["since", "limit"],
    },
    ToolSpec {
        name: "pr_list_backport_candidates",
        blurb: "Merged PRs labeled needs-backport",
        required: &["repo"],
        optional: &["label", "since", "limit"],
    },
    ToolSpec {
        name: "pr_is_docs_only",
        blurb: "Whether PR is docs-only changes",
        required: &["repo", "pr_number"],
        optional: &[],
    },
    ToolSpec {
        name: "pr_list_stale",
        blurb: "Stale open PRs",
        required: &["repo"],
        optional: &["days", "limit"],
    },
    ToolSpec {
        name: "pr_get_ci_snapshot",
        blurb: "CI_KIND + failing runs + digest per run (one call)",
        required: &["repo", "pr_number"],
        optional: &["max_runs", "include_external"],
    },
    ToolSpec {
        name: "ci_analyze_pr_failures",
        blurb: "Failing run IDs for a PR",
        required: &["repo", "pr_number"],
        optional: &[],
    },
    ToolSpec {
        name: "ci_get_run_summary",
        blurb: "Run status + failed job names",
        required: &["repo", "run_id"],
        optional: &[],
    },
    ToolSpec {
        name: "ci_get_failed_logs",
        blurb: "Distilled failure logs with synopsis (job/step/test/FP)",
        required: &["repo", "run_id"],
        optional: &["job_id", "focus", "offset_lines", "max_lines"],
    },
    ToolSpec {
        name: "ci_get_failure_digest",
        blurb: "Compact failure digest with verdict + excerpt",
        required: &["repo", "run_id"],
        optional: &["job_id"],
    },
    ToolSpec {
        name: "ci_list_runs",
        blurb: "List workflow runs for a branch",
        required: &["repo"],
        optional: &["branch", "limit"],
    },
    ToolSpec {
        name: "ci_branch_health",
        blurb: "Branch CI failure rate and streak",
        required: &["repo"],
        optional: &["branch", "limit"],
    },
    ToolSpec {
        name: "ci_workflow_stats",
        blurb: "Per-workflow CI stats on a branch",
        required: &["repo"],
        optional: &["branch", "limit", "top"],
    },
    ToolSpec {
        name: "ci_failure_fingerprint",
        blurb: "Failure fingerprint for flaky ledger matching",
        required: &["repo", "run_id"],
        optional: &[],
    },
    ToolSpec {
        name: "policy_classify_failure",
        blurb: "Rule-based failure class (test/infra/auth/timeout)",
        required: &["repo", "run_id"],
        optional: &[],
    },
    ToolSpec {
        name: "ci_compare_runs",
        blurb: "Compare two runs by fingerprint (no full logs)",
        required: &["repo", "run_id_a", "run_id_b"],
        optional: &[],
    },
    ToolSpec {
        name: "ci_correlate_prs",
        blurb: "Merged PRs before a failing run (regression-link)",
        required: &["repo", "run_id"],
        optional: &["window_days", "limit"],
    },
    ToolSpec {
        name: "ci_get_job_logs",
        blurb: "Distilled logs for one workflow job",
        required: &["repo", "run_id", "job_id"],
        optional: &["offset_lines", "max_lines"],
    },
    ToolSpec {
        name: "ci_list_workflows",
        blurb: "GitHub Actions workflow names and IDs",
        required: &["repo"],
        optional: &["limit"],
    },
    ToolSpec {
        name: "ci_list_external_checks",
        blurb: "External CI checks on a PR (not Actions)",
        required: &["repo", "pr_number"],
        optional: &[],
    },
    ToolSpec {
        name: "ci_get_check_url",
        blurb: "External check names with details URLs",
        required: &["repo", "pr_number"],
        optional: &[],
    },
    ToolSpec {
        name: "issue_list_open",
        blurb: "Open issues",
        required: &["repo"],
        optional: &["limit"],
    },
    ToolSpec {
        name: "issue_get",
        blurb: "Single issue details",
        required: &["repo", "issue_number"],
        optional: &[],
    },
    ToolSpec {
        name: "issue_search",
        blurb: "Search issues (GitHub query syntax)",
        required: &["repo", "query"],
        optional: &["limit"],
    },
    ToolSpec {
        name: "alert_list_open",
        blurb: "Dependabot / security alerts",
        required: &["repo"],
        optional: &["limit"],
    },
    ToolSpec {
        name: "alert_summarize_open",
        blurb: "Dependabot severity rollup",
        required: &["repo"],
        optional: &["limit"],
    },
    ToolSpec {
        name: "release_list_tags",
        blurb: "Recent git tags (newest first)",
        required: &["repo"],
        optional: &["limit"],
    },
    ToolSpec {
        name: "release_notes_draft",
        blurb: "Release-notes bullets from merged PRs since tag",
        required: &["repo"],
        optional: &["since_tag", "limit"],
    },
    ToolSpec {
        name: "notify_post_slack",
        blurb: "Post compact Slack message (mutating; needs webhook env)",
        required: &["text"],
        optional: &["webhook_url"],
    },
    ToolSpec {
        name: "event_list_recent",
        blurb: "Recent GitHub webhook events (HTTP mode ingest)",
        required: &[],
        optional: &["repo", "kind", "limit"],
    },
    ToolSpec {
        name: "store_get_latest_digest",
        blurb: "Latest local digest + approvals",
        required: &[],
        optional: &[],
    },
    ToolSpec {
        name: "store_list_pending_approvals",
        blurb: "Pending approval queue (no digest body)",
        required: &[],
        optional: &["limit"],
    },
    ToolSpec {
        name: "store_get_oncall_handoff",
        blurb: "On-call handoff pack from local Store",
        required: &[],
        optional: &[],
    },
    ToolSpec {
        name: "harness_run_workflow",
        blurb: "Run a built-in batch workflow by id (e.g. daily-work, review-radar)",
        required: &["workflow_id"],
        optional: &[],
    },
    ToolSpec {
        name: "harness_triage_pr",
        blurb: "Full PR triage workflow (same as TUI `t`) — updates Store triage_note",
        required: &["repo", "pr_number"],
        optional: &[],
    },
    ToolSpec {
        name: "harness_daily_digest",
        blurb: "Run daily-work workflow — publishes digest to Store",
        required: &[],
        optional: &[],
    },
    ToolSpec {
        name: "bash_run",
        blurb: "Run a shell command or short script after LLM safety review (no-approval path; prefer single-line for simple ops)",
        required: &["command"],
        optional: &["cwd"],
    },
    ToolSpec {
        name: "python_run",
        blurb: "Run a Python snippet after built-in LLM safety review (no-approval path)",
        required: &["code"],
        optional: &["cwd"],
    },
    ToolSpec {
        name: "web_fetch",
        blurb: "Fetch URL or local HTML — HTTP by default; pass browser:true for headless Chromium (JS/anti-bot)",
        required: &["url"],
        optional: &["mode", "max_chars", "browser"],
    },
    ToolSpec {
        name: "read_file",
        blurb: "Read file lines under chat.workspace (path + optional line range)",
        required: &["path"],
        optional: &["start_line", "max_lines"],
    },
    ToolSpec {
        name: "grep",
        blurb: "Search file contents with ripgrep under workspace path",
        required: &["pattern"],
        optional: &["path"],
    },
    ToolSpec {
        name: "glob",
        blurb: "Find files by glob pattern under workspace path",
        required: &["pattern"],
        optional: &["path"],
    },
    ToolSpec {
        name: "edit_file",
        blurb: "Replace old_string with new_string; unique match unless replace_all (LLM safety review)",
        required: &["path", "old_string", "new_string"],
        optional: &["replace_all"],
    },
    ToolSpec {
        name: "write_file",
        blurb: "Create or overwrite UTF-8 text file; preserves line endings on overwrite (LLM safety review)",
        required: &["path", "content"],
        optional: &["create_only"],
    },
    ToolSpec {
        name: "tool_list",
        blurb: "Lazy MCP: list all remote tools (prefer tool_search)",
        required: &[],
        optional: &[],
    },
    ToolSpec {
        name: "tool_list_category",
        blurb: "Lazy MCP: list tools in one category (CI, PR, …)",
        required: &["category"],
        optional: &[],
    },
    ToolSpec {
        name: "tool_search",
        blurb: "Lazy MCP: search tools by keyword",
        required: &["query"],
        optional: &["limit"],
    },
    ToolSpec {
        name: "tool_describe",
        blurb: "Lazy MCP: optional full schema for one tool",
        required: &["name"],
        optional: &[],
    },
    ToolSpec {
        name: "tool_call",
        blurb: "Lazy MCP: call business tool by name",
        required: &["name", "args"],
        optional: &[],
    },
    ToolSpec {
        name: "resource_read",
        blurb: "Read MCP resource URI (github://pull/.../ci-snapshot etc.)",
        required: &["uri"],
        optional: &[],
    },
    ToolSpec {
        name: "skill_load",
        blurb: "Load a technique skill body and warm its tools[] schemas",
        required: &["name"],
        optional: &[],
    },
    ToolSpec {
        name: "ci_rerun_workflow",
        blurb: "Mutating — harness queues approval on call",
        required: &["repo", "run_id"],
        optional: &[],
    },
    ToolSpec {
        name: "pr_create_backport",
        blurb: "Mutating — harness queues approval on call",
        required: &["repo", "pr_number", "target_branch"],
        optional: &[],
    },
    ToolSpec {
        name: "backport_get_conflict_files",
        blurb: "Conflict files in backport workspace",
        required: &["workspace_path"],
        optional: &[],
    },
    ToolSpec {
        name: "backport_suggest_resolution",
        blurb: "Backport conflict resolution hints",
        required: &["workspace_path"],
        optional: &["max_files"],
    },
    ToolSpec {
        name: "pr_post_comment",
        blurb: "Mutating — harness queues approval on call",
        required: &["repo", "pr_number", "body"],
        optional: &[],
    },
    ToolSpec {
        name: "issue_add_label",
        blurb: "Mutating — harness queues approval on call",
        required: &["repo", "issue_number", "label"],
        optional: &[],
    },
];

/// Static tool catalog for chat (schemas + harness nudges).
#[derive(Debug, Clone, Copy, Default)]
pub struct ToolCatalog;

fn followup_lines_to_try_steps(followup: &str) -> Vec<String> {
    followup
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("Args include"))
        .map(str::to_string)
        .collect()
}

impl ToolCatalog {
    pub fn new() -> Self {
        Self
    }

    /// Meta, mutating, catalog, harness, or any plausible MCP tool name.
    pub fn is_known_chat_tool(&self, name: &str) -> bool {
        if META_TOOLS.contains(&name) || MUTATING_TOOLS.contains(&name) {
            return true;
        }
        if spec_by_name(name).is_some() || harness_tools::is_chat_harness_tool(name) {
            return true;
        }
        // Reject compound hallucinations (e.g. pr_get_overview_and_friends).
        if let Some((base, _)) = self.salvage_hallucinated_tool_name(name) {
            if base != name {
                return false;
            }
        }
        is_plausible_tool_name(name)
    }

    pub fn suggest_tool_name(&self, raw: &str) -> Option<String> {
        let raw = raw.trim().trim_matches('`').to_ascii_lowercase();
        if raw.is_empty() {
            return None;
        }
        if self.is_known_chat_tool(&raw) {
            return Some(raw);
        }
        if let Some((base, _)) = self.salvage_hallucinated_tool_name(&raw) {
            if self.is_known_chat_tool(&base) {
                return Some(base);
            }
        }
        let mut best: Option<(String, usize)> = None;
        for name in self.candidate_names() {
            let mut score = common_prefix_len(&raw, &name);
            if raw.contains(name.as_str()) {
                score = score.max(name.len() + 50);
            }
            for token in name.split('_') {
                if token.len() >= 4 && raw.contains(token) {
                    score = score.max(token.len() + 10);
                }
            }
            if score >= 8 && best.as_ref().is_none_or(|(_, s)| score > *s) {
                best = Some((name, score));
            }
        }
        best.map(|(name, _)| name)
    }

    pub fn salvage_hallucinated_tool_name(&self, name: &str) -> Option<(String, Option<u32>)> {
        let name = name.trim().trim_matches('`');
        let prefixes = self.merged_prefixes();
        for prefix in &prefixes {
            if name == prefix.as_str() {
                return None;
            }
            if name.starts_with(prefix.as_str()) && name.len() > prefix.len() {
                let next = name.as_bytes().get(prefix.len()).copied();
                if next == Some(b'_') {
                    let suffix = &name[prefix.len()..];
                    let pr = pr_numbers_in_tool_suffix(suffix).into_iter().next();
                    return Some((prefix.clone(), pr));
                }
            }
        }
        if is_plausible_tool_name(name) {
            return None;
        }
        for prefix in &prefixes {
            if let Some(suffix) = name.strip_prefix(prefix.as_str()) {
                let pr = pr_numbers_in_tool_suffix(suffix).into_iter().next();
                return Some((prefix.clone(), pr));
            }
        }
        None
    }

    pub fn format_invalid_tool_nudge(&self, bad: &str) -> String {
        let suggestion = self.suggest_tool_name(bad);
        let mut out = format!(
            "Invalid tool_name `{bad}`. Call tools via the native API — do not invent combined names."
        );
        if let Some(tool) = suggestion {
            out.push_str(&format!(
                "\n\nDid you mean `{tool}`? ({})",
                self.tool_blurb(&tool)
            ));
            out.push_str(&self.format_tool_contract_block(&tool, None, None, None));
            if let Some(related) = self.related_tools(&tool) {
                out.push_str("\n\nRelated: ");
                out.push_str(&related);
            }
        } else {
            out.push_str("\n\n");
            out.push_str(&self.format_generic_contract());
        }
        out
    }

    pub fn format_unknown_tool_nudge(&self, bad: &str) -> String {
        let fallback = TOOLS.first().map(|t| t.name).unwrap_or("pr_list_open");
        let suggestion = self
            .suggest_tool_name(bad)
            .unwrap_or_else(|| fallback.to_string());
        format!(
            "Unknown tool_name `{bad}`. Did you mean `{suggestion}`? ({}){}",
            self.tool_blurb(&suggestion),
            self.format_tool_contract_block(&suggestion, None, None, None),
        )
    }

    pub fn format_tool_args_nudge(
        &self,
        tool_name: &str,
        missing_field: &str,
        example_value: Option<&str>,
        example_repo: Option<&str>,
    ) -> String {
        let mut out = format!(
            "Tool `{tool_name}` is missing required `{missing_field}` in tool_args \
(use tool_args, not params/args at the top level)."
        );
        out.push_str(&self.format_required_optional(tool_name));
        let pr = if missing_field == "pr_number" {
            example_value.and_then(|v| v.parse::<u32>().ok())
        } else {
            None
        };
        let run_id = if missing_field == "run_id" {
            example_value.and_then(|v| v.parse::<i64>().ok())
        } else {
            None
        };
        out.push_str(&self.format_tool_contract_block(tool_name, pr, run_id, example_repo));
        out
    }

    /// True when `tool_args` already has a non-empty value for a schema field name.
    pub fn tool_arg_field_satisfied(tool_args: &Value, field: &str) -> bool {
        match field {
            "repo" => tool_args
                .get("repo")
                .and_then(Value::as_str)
                .is_some_and(|s| !s.trim().is_empty()),
            "pr_number" => tool_args
                .get("pr_number")
                .and_then(|v| {
                    v.as_u64()
                        .or_else(|| v.as_i64().filter(|n| *n >= 0).map(|n| n as u64))
                        .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
                })
                .is_some(),
            "run_id" => tool_args
                .get("run_id")
                .and_then(|v| {
                    v.as_i64()
                        .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
                })
                .is_some_and(|n| n >= 100_000),
            "name" => tool_args
                .get("name")
                .and_then(Value::as_str)
                .is_some_and(|s| !s.trim().is_empty()),
            "args" => tool_args.get("args").is_some_and(Value::is_object),
            other => tool_args.get(other).is_some_and(|v| !v.is_null()),
        }
    }

    /// Required schema fields that are absent or empty in `tool_args`.
    pub fn missing_required_fields(&self, tool_name: &str, tool_args: &Value) -> Vec<String> {
        let (required, _) = self.resolved_args(tool_name);
        required
            .into_iter()
            .filter(|field| !Self::tool_arg_field_satisfied(tool_args, field))
            .map(str::to_string)
            .collect()
    }

    /// After a failed tool call: unified harness envelope + args/schema context.
    pub fn format_tool_failure_nudge(
        &self,
        tool_name: &str,
        tool_args: &Value,
        error_body: &str,
        configured_repos: &[String],
    ) -> String {
        let pr = tool_args
            .get("pr_number")
            .and_then(|v| v.as_u64())
            .map(|n| n as u32)
            .or_else(|| {
                tool_args
                    .get("pr_number")
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.parse().ok())
            });
        let run_id = tool_args
            .get("run_id")
            .and_then(|v| v.as_i64())
            .or_else(|| {
                tool_args
                    .get("run_id")
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.parse().ok())
            });
        let repo = tool_args
            .get("repo")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty());
        let sent =
            serde_json::to_string_pretty(tool_args).unwrap_or_else(|_| tool_args.to_string());
        let missing = self.missing_required_fields(tool_name, tool_args);

        if error_body.contains("HARN:TOOL_FAILED")
            && error_body.contains("[Harness]")
            && (error_body.contains("ERROR:") || error_body.lines().count() > 6)
        {
            let core = error_body
                .strip_prefix("tool error: ")
                .unwrap_or(error_body);
            let mut out = core.to_string();
            out.push_str(&format!("\n\nArgs sent:\n{sent}"));
            out.push_str(&self.format_required_optional(tool_name));
            if !missing.is_empty() {
                out.push_str(&self.format_tool_contract_block(tool_name, pr, run_id, repo));
            }
            return out;
        }

        let envelope = if !missing.is_empty() {
            self.missing_args_envelope(tool_name, &missing, tool_args, pr, run_id, repo)
        } else if tool_name == BASH_RUN_TOOL {
            Self::bash_failure_envelope(tool_args, error_body)
        } else if file_tools::is_file_tool(tool_name) {
            file_tool_failure_envelope(tool_name, error_body)
        } else {
            self.mcp_failure_envelope(tool_name, tool_args, error_body, configured_repos)
        };

        let mut out = envelope.format_harness_nudge();
        out.push_str(&format!("\n\nArgs sent:\n{sent}"));
        out.push_str(&self.format_required_optional(tool_name));
        if !missing.is_empty() {
            out.push_str(&self.format_tool_contract_block(tool_name, pr, run_id, repo));
        }
        out
    }

    fn bash_failure_envelope(tool_args: &Value, error_body: &str) -> ErrorEnvelope {
        let command = tool_args
            .get("command")
            .and_then(|v| v.as_str())
            .unwrap_or("?");
        if error_body.contains("bash_run:") && bash_tool::output_indicates_failure(error_body) {
            return bash_exit_failure_envelope(command, error_body);
        }
        bash_validation_envelope(error_body, Some(command))
    }

    fn missing_args_envelope(
        &self,
        tool_name: &str,
        missing: &[String],
        tool_args: &Value,
        pr: Option<u32>,
        run_id: Option<i64>,
        repo: Option<&str>,
    ) -> ErrorEnvelope {
        let fields = missing.join(", ");
        let example =
            serde_json::from_str::<Value>(&example_native_tool_args(tool_name, pr, run_id, repo))
                .ok()
                .map(|args| {
                    serde_json::json!({
                        "name": tool_name,
                        "arguments": args
                    })
                });
        generic_tool_failure_envelope(
            tool_name,
            &format!("Tool `{tool_name}` is missing required arguments"),
            &format!("Missing: {fields}"),
            vec![format!("Add required field(s): {fields}")],
            example,
            &format!(
                "Args sent: {}",
                serde_json::to_string(tool_args).unwrap_or_default()
            ),
        )
        .with_code("TOOL_MISSING_ARGS")
    }

    fn mcp_failure_envelope(
        &self,
        tool_name: &str,
        tool_args: &Value,
        error_body: &str,
        configured_repos: &[String],
    ) -> ErrorEnvelope {
        let code = classify_github_error_code(error_body);
        let parsed = crate::agent::harness_errors::parse_error_line(error_body);
        let why = parsed
            .as_ref()
            .map(|p| {
                if p.hint.is_empty() {
                    p.message.clone()
                } else {
                    format!("{} — {}", p.message, p.hint)
                }
            })
            .unwrap_or_else(|| {
                error_body
                    .lines()
                    .find(|l| !l.trim().is_empty())
                    .unwrap_or(error_body)
                    .trim()
                    .to_string()
            });
        let followup = self.format_actionable_failure_followup(
            tool_name,
            tool_args,
            error_body,
            configured_repos,
        );
        let try_steps = followup_lines_to_try_steps(&followup);
        let pr = tool_args
            .get("pr_number")
            .and_then(|v| v.as_u64())
            .map(|n| n as u32);
        let run_id = tool_args.get("run_id").and_then(|v| v.as_i64());
        let repo = tool_args.get("repo").and_then(|v| v.as_str());
        let example =
            serde_json::from_str::<Value>(&example_native_tool_args(tool_name, pr, run_id, repo))
                .ok()
                .map(|args| {
                    serde_json::json!({
                        "name": tool_name,
                        "arguments": args
                    })
                });
        let mut env = generic_tool_failure_envelope(
            tool_name,
            &format!("Tool `{tool_name}` call failed"),
            &why,
            try_steps,
            example,
            error_body,
        );
        env.code = code;
        env
    }

    fn format_actionable_failure_followup(
        &self,
        tool_name: &str,
        tool_args: &Value,
        error_body: &str,
        configured_repos: &[String],
    ) -> String {
        let low = error_body.to_ascii_lowercase();
        let err_code = crate::agent::harness_errors::parse_error_line(error_body)
            .map(|p| p.code)
            .or_else(|| {
                error_body
                    .lines()
                    .next()
                    .and_then(|l| l.strip_prefix("ERROR:"))
                    .map(|rest| rest.split('|').next().unwrap_or("").trim().to_string())
            });
        let repo = tool_args
            .get("repo")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        let mut out = String::from(
            "Args include all required fields — this is not a missing-parameter error.",
        );
        if err_code.as_deref() == Some("NOT_FOUND")
            || low.contains("not found")
            || low.contains("http 404")
            || low.contains("could not resolve to a repository")
        {
            out.push_str(
                "\nGitHub could not find the repo, PR, workflow run, or log for those IDs.",
            );
            if !configured_repos.is_empty() {
                out.push_str("\nConfigured repos: ");
                out.push_str(&configured_repos.join(", "));
                if !repo.is_empty()
                    && !configured_repos
                        .iter()
                        .any(|r| r.eq_ignore_ascii_case(repo))
                {
                    out.push_str(&format!(
                        "\n`{repo}` is not in that list — pick a configured repo or update coworker.yaml."
                    ));
                }
            }
            if matches!(
                tool_name,
                "ci_get_failed_logs" | "ci_get_run_summary" | "ci_failure_fingerprint"
            ) {
                out.push_str(
                    "\nConfirm `run_id` came from `ci_analyze_pr_failures` or `ci_get_run_summary` for the same `repo`.",
                );
            }
            if tool_name == "ci_get_failed_logs" && low.contains("log not found") {
                out.push_str(
                    "\nLogs may be pending or expired — try `ci_get_run_summary` before fetching failed logs.",
                );
            }
        } else if err_code.as_deref() == Some("TRANSIENT")
            || err_code.as_deref() == Some("RATE_LIMIT")
            || low.contains("temporary server error")
            || low.contains("http 504")
            || low.contains("http 503")
            || low.contains("http 502")
            || low.contains("gateway timeout")
        {
            if err_code.as_deref() == Some("RATE_LIMIT") || low.contains("rate limit") {
                out.push_str("\nGitHub rate limit — wait at least a minute before retrying.");
            } else {
                out.push_str("\nTransient GitHub error — retry the same call once.");
            }
        } else if err_code.as_deref() == Some("FORBIDDEN")
            || err_code.as_deref() == Some("AUTH")
            || low.contains("permission")
            || low.contains("http 403")
            || low.contains("forbidden")
            || low.contains("authentication")
        {
            out.push_str(
                "\nPermission or auth error — check GH_TOKEN / `gh auth login` for this repo.",
            );
        } else if err_code.as_deref() == Some("EXTERNAL_CI") {
            out.push_str("\nExternal CI failure — use `ci_list_external_checks`; do not call ci_get_failed_logs.");
        } else {
            out.push_str(
                "\nFix the underlying error (wrong IDs, repo, or permissions) or try another tool — required args were already sent.",
            );
        }
        out
    }

    fn format_tool_contract_block(
        &self,
        tool_name: &str,
        pr_number: Option<u32>,
        run_id: Option<i64>,
        example_repo: Option<&str>,
    ) -> String {
        let example = example_native_tool_args(tool_name, pr_number, run_id, example_repo);
        format!(
            "\n\nCall `{tool_name}` via the native tool API with arguments like:\n{example}\n\n\
             Read-only tools may be batched in one turn; mutating tools need approval and run one at a time."
        )
    }

    fn format_generic_contract(&self) -> String {
        let example_tool = TOOLS.first().map(|t| t.name).unwrap_or("pr_list_open");
        format!(
            "Call `{example_tool}` via the native tool API with arguments like:\n{}\n\
             You may call multiple read-only tools in one turn; mutating tools one at a time with approval.",
            example_native_tool_args(example_tool, None, None, None)
        )
    }

    fn format_required_optional(&self, tool_name: &str) -> String {
        let (required, optional) = self.resolved_args(tool_name);
        let req = if required.is_empty() {
            "(none)".to_string()
        } else {
            required.join(", ")
        };
        let opt = if optional.is_empty() {
            "(none)".to_string()
        } else {
            optional.join(", ")
        };
        format!("\nRequired tool_args: {req}\nOptional: {opt}")
    }

    fn tool_blurb(&self, name: &str) -> Cow<'static, str> {
        spec_by_name(name)
            .map(|s| Cow::Borrowed(s.blurb))
            .unwrap_or(Cow::Borrowed("MCP tool — see TOOLS.md or tool_describe"))
    }

    fn resolved_args(&self, name: &str) -> (Vec<&'static str>, Vec<&'static str>) {
        if let Some(spec) = spec_by_name(name) {
            return (spec.required.to_vec(), spec.optional.to_vec());
        }
        let (req, opt) = inferred_spec_fields(name);
        (req.to_vec(), opt.to_vec())
    }

    fn related_tools(&self, tool: &str) -> Option<String> {
        let related: &[&str] = match tool {
            "pr_get_overview" => &["pr_list_changed_files", "pr_get_diff", "pr_list_open"],
            "pr_list_changed_files" => &["pr_diff_risk_scan", "pr_get_diff", "pr_get_overview"],
            "ci_analyze_pr_failures" => &[
                "pr_get_ci_snapshot",
                "ci_get_run_summary",
                "ci_failure_fingerprint",
                "ci_get_failed_logs",
            ],
            "pr_get_ci_snapshot" => &["ci_get_failed_logs", "ci_rerun_workflow", "ci_compare_runs"],
            "ci_get_run_summary" => &[
                "ci_get_failure_digest",
                "ci_get_failed_logs",
                "ci_get_job_logs",
                "ci_correlate_prs",
            ],
            "ci_get_failure_digest" => &[
                "ci_get_failed_logs",
                "policy_classify_failure",
                "ci_rerun_workflow",
            ],
            "ci_get_failed_logs" => &["policy_classify_failure", "ci_get_job_logs"],
            "ci_failure_fingerprint" => &[
                "ci_get_failure_digest",
                "policy_classify_failure",
                "ci_compare_runs",
                "ci_get_failed_logs",
            ],
            "policy_classify_failure" => &[
                "ci_rerun_workflow",
                "pr_draft_ci_comment",
                "ci_get_failed_logs",
            ],
            "ci_list_runs" => &[
                "ci_branch_health",
                "ci_get_run_summary",
                "ci_list_workflows",
            ],
            "ci_list_workflows" => &["ci_list_runs", "ci_branch_health", "ci_workflow_stats"],
            "ci_branch_health" => &[
                "ci_workflow_stats",
                "ci_get_run_summary",
                "ci_correlate_prs",
            ],
            "ci_workflow_stats" => &["ci_branch_health", "ci_list_runs"],
            "release_list_tags" => &["release_notes_draft", "pr_list_merged"],
            "pr_get_merge_blockers" => &["pr_get_review_state", "pr_list_merge_blocked"],
            "pr_list_merge_ready" => &["pr_list_merge_blocked"],
            "pr_list_merge_blocked" => &["pr_get_merge_blockers"],
            "pr_diff_risk_scan" => &["pr_get_diff", "pr_list_large"],
            "pr_list_large" => &["pr_diff_risk_scan", "pr_get_overview"],
            "pr_get_review_routing" => &["pr_get_review_state", "pr_list_changed_files"],
            "ci_list_external_checks" => &["ci_get_check_url"],
            "ci_get_check_url" => &["ci_list_external_checks"],
            "backport_get_conflict_files" => &["backport_suggest_resolution"],
            "backport_suggest_resolution" => &["backport_get_conflict_files"],
            "alert_list_open" => &["alert_summarize_open"],
            "alert_summarize_open" => &["alert_list_open"],
            "ci_rerun_workflow" => &["ci_compare_runs"],
            "pr_list_open" => &[
                "pr_get_overview",
                "pr_list_waiting_review",
                "pr_get_status_batch",
            ],
            "pr_list_waiting_review" => &["pr_get_status_batch", "pr_get_overview_batch"],
            "pr_get_status_batch" => &["pr_get_overview", "pr_get_overview_batch"],
            "pr_get_overview_batch" => &["pr_get_overview", "ci_analyze_pr_failures"],
            _ => return None,
        };
        let names: Vec<String> = related
            .iter()
            .filter(|t| self.is_known_chat_tool(t))
            .map(|t| format!("`{t}`"))
            .collect();
        if names.is_empty() {
            None
        } else {
            Some(names.join(", "))
        }
    }

    fn candidate_names(&self) -> Vec<String> {
        list_github_tool_names()
    }

    fn merged_prefixes(&self) -> Vec<String> {
        let mut names = self.candidate_names();
        names.sort_by_key(|p| std::cmp::Reverse(p.len()));
        names
    }

    /// OpenAI/Ollama-native `tools` array for the chat LLM API (empty warmup set).
    #[allow(dead_code)] // convenience wrapper; chat uses `native_tool_definitions_for_session`.
    pub fn native_tool_definitions(&self, mode: ChatToolMode) -> Vec<Value> {
        self.native_tool_definitions_for_session(mode, &HashSet::new())
    }

    /// Lazy/auto mode: minimal preload + session-warmed business tools.
    pub fn native_tool_definitions_for_session(
        &self,
        mode: ChatToolMode,
        warmed: &HashSet<String>,
    ) -> Vec<Value> {
        TOOLS
            .iter()
            .filter(|spec| match mode {
                ChatToolMode::Native => true,
                ChatToolMode::Auto | ChatToolMode::Lazy => {
                    is_lazy_native_tool(spec.name) || warmed.contains(spec.name)
                }
            })
            .map(native_tool_from_spec)
            .collect()
    }
}

/// All GitHub harness tool names from the static catalog.
pub fn list_github_tool_names() -> Vec<String> {
    TOOLS.iter().map(|spec| spec.name.to_string()).collect()
}

pub fn tool_blurb_for_name(name: &str) -> &'static str {
    spec_by_name(name)
        .map(|s| s.blurb)
        .unwrap_or("GitHub harness tool")
}

pub fn tool_fields_for_name(name: &str) -> (Vec<&'static str>, Vec<&'static str>) {
    if let Some(spec) = spec_by_name(name) {
        return (spec.required.to_vec(), spec.optional.to_vec());
    }
    let (req, opt) = inferred_spec_fields(name);
    (req.to_vec(), opt.to_vec())
}

/// Tools always exposed as native schemas in lazy/auto chat mode.
pub fn is_lazy_native_tool(name: &str) -> bool {
    PRELOAD_NATIVE_TOOLS.contains(&name)
}

/// True when `name` has a catalog entry (business or meta tool spec).
pub fn is_catalog_tool(name: &str) -> bool {
    spec_by_name(name).is_some()
}

/// Whether a business tool accepts a `repo` parameter.
pub fn tool_accepts_repo(name: &str) -> bool {
    let (required, optional) = resolved_tool_fields(name);
    required.contains(&"repo") || optional.contains(&"repo")
}

/// Whether a business tool accepts `pr_number`.
pub fn tool_accepts_pr_number(name: &str) -> bool {
    let (required, optional) = resolved_tool_fields(name);
    required.contains(&"pr_number") || optional.contains(&"pr_number")
}

fn resolved_tool_fields(name: &str) -> (Vec<&'static str>, Vec<&'static str>) {
    if let Some(spec) = spec_by_name(name) {
        return (spec.required.to_vec(), spec.optional.to_vec());
    }
    let (req, opt) = inferred_spec_fields(name);
    (req.to_vec(), opt.to_vec())
}

fn json_type_for_arg(key: &str) -> &'static str {
    match key {
        "repo" | "author" | "branch" | "body" | "target_branch" | "since" | "name" | "query"
        | "category" | "uri" | "command" | "cwd" | "path" | "pattern" | "old_string"
        | "new_string" | "content" | "url" | "mode" | "code" => "string",
        "replace_all" | "create_only" | "browser" => "boolean",
        "args" => "object",
        _ => "integer",
    }
}

fn tool_parameters(spec: &ToolSpec) -> Value {
    let mut props = serde_json::Map::new();
    let mut required = Vec::new();
    for key in spec.required {
        props.insert(
            (*key).to_string(),
            serde_json::json!({ "type": json_type_for_arg(key) }),
        );
        required.push(*key);
    }
    for key in spec.optional {
        if !props.contains_key(*key) {
            props.insert(
                (*key).to_string(),
                serde_json::json!({ "type": json_type_for_arg(key) }),
            );
        }
    }
    if spec.name == "tool_call" {
        props.insert("name".to_string(), serde_json::json!({ "type": "string" }));
        props.insert("args".to_string(), serde_json::json!({ "type": "object" }));
        if !required.contains(&"name") {
            required.push("name");
        }
    }
    serde_json::json!({
        "type": "object",
        "properties": props,
        "required": required,
        "additionalProperties": false,
    })
}

fn native_tool_from_spec(spec: &ToolSpec) -> Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": spec.name,
            "description": spec.blurb,
            "parameters": tool_parameters(spec),
        }
    })
}

pub fn is_plausible_tool_name(name: &str) -> bool {
    let name = name.trim().trim_matches('`');
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

fn spec_by_name(name: &str) -> Option<&'static ToolSpec> {
    TOOLS.iter().find(|t| t.name == name)
}

fn inferred_spec_fields(name: &str) -> (&'static [&'static str], &'static [&'static str]) {
    if harness_tools::is_chat_harness_tool(name) || name == "tool_list" {
        return (&[], &[]);
    }
    if matches!(name, "read_file") {
        return (&["path"], &["start_line", "max_lines"]);
    }
    if matches!(name, "grep") {
        return (&["pattern"], &["path"]);
    }
    if matches!(name, "glob") {
        return (&["pattern"], &["path"]);
    }
    if matches!(name, "edit_file") {
        return (&["path", "old_string", "new_string"], &["replace_all"]);
    }
    if matches!(name, "write_file") {
        return (&["path", "content"], &["create_only"]);
    }
    if matches!(name, "tool_describe") {
        return (&["name"], &[]);
    }
    if matches!(name, "tool_call") {
        return (&["name", "args"], &[]);
    }
    if name == "ci_get_failed_logs" {
        return (
            &["repo", "run_id"],
            &["job_id", "focus", "offset_lines", "max_lines"],
        );
    }
    if name == "ci_get_failure_digest" {
        return (&["repo", "run_id"], &["job_id"]);
    }
    if name == "ci_get_run_summary" {
        return (&["repo", "run_id"], &[]);
    }
    if name == "ci_list_runs" {
        return (&["repo"], &["branch", "limit"]);
    }
    if name == "ci_list_workflows" {
        return (&["repo"], &["limit"]);
    }
    if name == "ci_branch_health" {
        return (&["repo"], &["branch", "limit"]);
    }
    if name == "ci_workflow_stats" {
        return (&["repo"], &["branch", "limit", "top"]);
    }
    if name == "pr_list_merge_ready" || name == "pr_list_merge_blocked" {
        return (&["repo"], &["limit"]);
    }
    if name == "pr_draft_ci_comment" {
        return (&["repo", "pr_number", "run_id"], &[]);
    }
    if name == "ci_failure_fingerprint" {
        return (&["repo", "run_id"], &[]);
    }
    if name == "policy_classify_failure" {
        return (&["repo", "run_id"], &[]);
    }
    if name == "ci_compare_runs" {
        return (&["repo", "run_id_a", "run_id_b"], &[]);
    }
    if name == "ci_correlate_prs" {
        return (&["repo", "run_id"], &["window_days", "limit"]);
    }
    if name == "ci_get_job_logs" {
        return (
            &["repo", "run_id", "job_id"],
            &["offset_lines", "max_lines"],
        );
    }
    if name == "ci_list_external_checks" || name == "ci_get_check_url" {
        return (&["repo", "pr_number"], &[]);
    }
    if name == "pr_list_open" {
        return (&["repo"], &["author", "limit"]);
    }
    if name == "repo_get_info" {
        return (&["repo"], &["label_limit"]);
    }
    if name == "pr_list_waiting_review" {
        return (&["repo"], &["limit"]);
    }
    if name == "pr_get_status_batch" || name == "pr_get_overview_batch" {
        return (&["repo", "pr_numbers"], &[]);
    }
    if name == "pr_list_stale" {
        return (&["repo"], &["days", "limit"]);
    }
    if name == "pr_list_merged" {
        return (&["repo"], &["since", "label", "limit"]);
    }
    if name == "pr_get_diff" {
        return (&["repo", "pr_number"], &["path", "max_bytes"]);
    }
    if name == "issue_list_open" || name == "alert_list_open" || name == "alert_summarize_open" {
        return (&["repo"], &["limit"]);
    }
    if name == "issue_get" {
        return (&["repo", "issue_number"], &[]);
    }
    if name == "issue_search" {
        return (&["repo", "query"], &["limit"]);
    }
    if name == "pr_list_backport_candidates" {
        return (&["repo"], &["label", "since", "limit"]);
    }
    if name == "release_list_tags" {
        return (&["repo"], &["limit"]);
    }
    if name == "release_notes_draft" {
        return (&["repo"], &["since_tag", "limit"]);
    }
    if name == "issue_add_label" {
        return (&["repo", "issue_number", "label"], &[]);
    }
    if name == "notify_post_slack" {
        return (&["text"], &["webhook_url"]);
    }
    if name == "event_list_recent" {
        return (&[], &["repo", "kind", "limit"]);
    }
    if name == "backport_get_conflict_files" {
        return (&["workspace_path"], &[]);
    }
    if name == "backport_suggest_resolution" {
        return (&["workspace_path"], &["max_files"]);
    }
    if name == "pr_list_large" {
        return (&["repo"], &["min_files", "min_lines", "limit"]);
    }
    if name == "pr_get_review_state" || name == "pr_diff_risk_scan" {
        return (&["repo", "pr_number"], &[]);
    }
    if name.starts_with("pr_list_") || name == "ci_list_runs" || name == "ci_list_workflows" {
        return (&["repo"], &[]);
    }
    if name.starts_with("pr_") || name.starts_with("ci_analyze") {
        return (&["repo", "pr_number"], &[]);
    }
    if MUTATING_TOOLS.contains(&name) {
        return match name {
            "ci_rerun_workflow" => (&["repo", "run_id"] as &[_], &[] as &[_]),
            "pr_post_comment" => (&["repo", "pr_number", "body"], &[]),
            "pr_create_backport" => (&["repo", "pr_number", "target_branch"], &[]),
            _ => (&["repo"], &[]),
        };
    }
    (&["repo"], &[])
}

fn example_native_tool_args(
    tool_name: &str,
    pr_number: Option<u32>,
    run_id: Option<i64>,
    example_repo: Option<&str>,
) -> String {
    let pr = pr_number.unwrap_or(19263);
    let run = run_id.unwrap_or(12_345_678);
    let repo = example_repo.unwrap_or("owner/repo");
    match tool_name {
        "pr_get_overview"
        | "pr_get_status"
        | "pr_get_merge_blockers"
        | "pr_list_changed_files"
        | "pr_get_ci_snapshot"
        | "ci_analyze_pr_failures" => format!("{{\"repo\":\"{repo}\",\"pr_number\":{pr}}}"),
        "pr_get_diff" => {
            format!("{{\"repo\":\"{repo}\",\"pr_number\":{pr},\"path\":\"src/lib.rs\"}}")
        }
        "pr_list_open" => format!(r#"{{"repo":"{repo}","author":"@me","limit":20}}"#),
        "repo_get_info" => format!(r#"{{"repo":"{repo}"}}"#),
        "pr_list_waiting_review"
        | "pr_list_merge_ready"
        | "pr_list_merge_blocked"
        | "pr_list_merged"
        | "pr_list_stale"
        | "issue_list_open"
        | "alert_list_open"
        | "alert_summarize_open"
        | "ci_list_runs"
        | "ci_list_workflows"
        | "release_list_tags" => format!("{{\"repo\":\"{repo}\"}}"),
        "release_notes_draft" => format!(r#"{{"repo":"{repo}","since_tag":"v1.0.0"}}"#),
        "ci_get_run_summary"
        | "ci_get_failed_logs"
        | "ci_get_failure_digest"
        | "ci_failure_fingerprint"
        | "ci_correlate_prs" => {
            format!("{{\"repo\":\"{repo}\",\"run_id\":{run}}}")
        }
        "ci_get_job_logs" => {
            format!("{{\"repo\":\"{repo}\",\"run_id\":{run},\"job_id\":999}}")
        }
        "pr_draft_ci_comment" => {
            format!("{{\"repo\":\"{repo}\",\"pr_number\":{pr},\"run_id\":{run}}}")
        }
        "ci_compare_runs" => {
            format!(
                "{{\"repo\":\"{repo}\",\"run_id_a\":{run},\"run_id_b\":{}}}",
                run + 1
            )
        }
        "ci_list_external_checks" | "ci_get_check_url" | "pr_get_review_routing" => {
            format!("{{\"repo\":\"{repo}\",\"pr_number\":{pr}}}")
        }
        "pr_get_status_batch" => format!(r#"{{"repo":"{repo}","pr_numbers":"{pr}"}}"#),
        "pr_get_overview_batch" => format!(r#"{{"repo":"{repo}","pr_numbers":"{pr},99"}}"#),
        "issue_get" => format!(r#"{{"repo":"{repo}","issue_number":42}}"#),
        "store_get_latest_digest"
        | "store_list_pending_approvals"
        | "store_get_oncall_handoff"
        | "tool_list" => "{}".to_string(),
        "tool_describe" => r#"{"name":"pr_get_overview"}"#.to_string(),
        "tool_call" => format!(r#"{{"name":"pr_list_open","args":{{"repo":"{repo}"}}}}"#),
        "ci_rerun_workflow" => format!("{{\"repo\":\"{repo}\",\"run_id\":{run}}}"),
        "pr_post_comment" => {
            format!("{{\"repo\":\"{repo}\",\"pr_number\":{pr},\"body\":\"…\"}}")
        }
        "pr_create_backport" => {
            format!("{{\"repo\":\"{repo}\",\"pr_number\":{pr},\"target_branch\":\"release/3.x\"}}")
        }
        other => format!("{{\"repo\":\"{repo}\"}} /* {other} */"),
    }
}

#[allow(dead_code)]
fn example_tool_json(
    tool_name: &str,
    pr_number: Option<u32>,
    run_id: Option<i64>,
    example_repo: Option<&str>,
) -> String {
    let args = example_native_tool_args(tool_name, pr_number, run_id, example_repo);
    format!("{{\"action\":\"tool\",\"tool_name\":\"{tool_name}\",\"tool_args\":{args}}}")
}

fn common_prefix_len(a: &str, b: &str) -> usize {
    a.chars().zip(b.chars()).take_while(|(x, y)| x == y).count()
}

fn pr_numbers_in_tool_suffix(suffix: &str) -> Vec<u32> {
    let mut out = Vec::new();
    let bytes = suffix.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            if i - start >= 3 {
                if let Ok(n) = suffix[start..i].parse::<u32>() {
                    if n > 0 {
                        out.push(n);
                    }
                }
            }
        } else {
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggest_compound_overview_name() {
        assert_eq!(
            ToolCatalog::new()
                .suggest_tool_name("pr_get_overview_and_changed_files_combined_for_prs_19264"),
            Some("pr_get_overview".to_string())
        );
    }

    #[test]
    fn salvage_extracts_pr_from_compound_name() {
        let (name, pr) = ToolCatalog::new()
            .salvage_hallucinated_tool_name(
                "pr_get_overview_and_changed_files_combined_for_prs_19264_19263",
            )
            .unwrap();
        assert_eq!(name, "pr_get_overview");
        assert_eq!(pr, Some(19264));
    }

    #[test]
    fn invalid_nudge_includes_suggestion_and_native_args() {
        let msg = ToolCatalog::new()
            .format_invalid_tool_nudge("pr_get_overview_and_changed_files_combined");
        assert!(msg.contains("Did you mean `pr_get_overview`"));
        assert!(msg.contains("Call `pr_get_overview`"));
        assert!(msg.contains("pr_number"));
        assert!(!msg.contains("Session preferred_tools"));
    }

    #[test]
    fn args_nudge_uses_concrete_repo_in_example_json() {
        let msg = ToolCatalog::new().format_tool_args_nudge(
            "pr_get_overview",
            "pr_number",
            Some("19272"),
            Some("acme/widget"),
        );
        assert!(msg.contains("acme/widget"));
        assert!(msg.contains("19272"));
        assert!(!msg.contains("owner/repo"));
    }

    #[test]
    fn args_nudge_includes_schema() {
        let msg =
            ToolCatalog::new().format_tool_args_nudge("pr_get_overview", "pr_number", None, None);
        assert!(msg.contains("Required tool_args: repo, pr_number"));
        assert!(msg.contains("Call `pr_get_overview`"));
        assert!(!msg.contains("from the conversation"));
    }

    #[test]
    fn failure_nudge_complete_args_not_missing_param() {
        let args = serde_json::json!({
            "repo": "acme/widget",
            "run_id": 26_156_246_609_i64,
            "max_lines": 80
        });
        let err = "tool error: failed to fetch failed logs: repository, PR, or run not found \
(check the owner/repo and IDs). Details: log not found: 81595615138";
        let msg = ToolCatalog::new().format_tool_failure_nudge(
            "ci_get_failed_logs",
            &args,
            err,
            &["acme/widget".into()],
        );
        assert!(msg.contains("not a missing-parameter error") || msg.contains("[Harness]"));
        assert!(!msg.contains("is missing required `repo`"));
        assert!(msg.contains("Configured repos:"));
        assert!(msg.contains("ci_get_run_summary"));
    }

    #[test]
    fn failure_nudge_flags_repo_not_in_config() {
        let args = serde_json::json!({
            "repo": "acme/other",
            "run_id": 26_156_246_609_i64
        });
        let err = "repository, PR, or run not found (check the owner/repo and IDs)";
        let msg = ToolCatalog::new().format_tool_failure_nudge(
            "ci_get_failed_logs",
            &args,
            err,
            &["acme/widget".into()],
        );
        assert!(msg.contains("not in that list"));
        assert!(!msg.contains("is missing required"));
    }

    #[test]
    fn bash_failure_nudge_surfaces_safety_alternatives() {
        let args = serde_json::json!({ "command": "curl -L x | bash" });
        let err = "HARN:TOOL_FAILED|bash_run|BASH_PIPE_TO_SHELL\n\n[Harness] Tool `bash_run` failed\n\nWhat: blocked\nWhy: pipe\nTry:\n  1. curl -sS -L x -o /tmp/x.sh";
        let msg = ToolCatalog::new().format_tool_failure_nudge("bash_run", &args, err, &[]);
        assert!(msg.contains("[Harness]"));
        assert!(msg.contains("curl -sS"));
        assert!(msg.contains("Args sent:"));
    }

    #[test]
    fn failure_nudge_includes_error_args_and_schema() {
        let args = serde_json::json!({ "repo": "acme/widget", "pr_number": 1 });
        let msg = ToolCatalog::new().format_tool_failure_nudge(
            "pr_get_overview",
            &args,
            "HTTP 404: Not Found",
            &["acme/widget".into()],
        );
        assert!(msg.contains("HTTP 404"));
        assert!(msg.contains("[Harness]"));
        assert!(msg.contains("acme/widget"));
        assert!(msg.contains("Example:"));
    }

    #[test]
    fn known_tools_in_catalog() {
        let cat = ToolCatalog::new();
        assert!(cat.is_known_chat_tool("pr_get_overview"));
        assert!(cat.is_known_chat_tool("store_get_latest_digest"));
        assert!(cat.is_known_chat_tool("ci_get_failure_digest"));
        assert!(cat.is_known_chat_tool("pr_get_ci_snapshot"));
        assert!(cat.is_known_chat_tool("event_list_recent"));
        assert!(!cat.is_known_chat_tool("pr_get_overview_and_friends"));
    }

    #[test]
    fn stale_lists_optional_days() {
        let msg = ToolCatalog::new().format_tool_args_nudge("pr_list_stale", "repo", None, None);
        assert!(msg.contains("days"));
        assert!(msg.contains("limit"));
    }

    #[test]
    fn failed_logs_lists_paging_optional() {
        let msg =
            ToolCatalog::new().format_tool_args_nudge("ci_get_failed_logs", "run_id", None, None);
        assert!(msg.contains("offset_lines"));
        assert!(msg.contains("max_lines"));
    }

    #[test]
    fn native_tool_definitions_cover_catalog() {
        let defs = ToolCatalog::new().native_tool_definitions(ChatToolMode::Native);
        assert_eq!(defs.len(), TOOLS.len());
    }

    #[test]
    fn warmed_tools_gain_native_schema() {
        let mut warmed = HashSet::new();
        warmed.insert("pr_get_overview".to_string());
        let defs =
            ToolCatalog::new().native_tool_definitions_for_session(ChatToolMode::Auto, &warmed);
        let names: Vec<String> = defs
            .iter()
            .filter_map(|d| d.get("function")?.get("name")?.as_str().map(str::to_string))
            .collect();
        assert!(names.contains(&"pr_get_overview".to_string()));
        assert!(names.contains(&"tool_search".to_string()));
    }

    #[test]
    fn lazy_native_tool_definitions_are_minimal() {
        let defs = ToolCatalog::new().native_tool_definitions(ChatToolMode::Lazy);
        let names: Vec<String> = defs
            .iter()
            .filter_map(|d| d.get("function")?.get("name")?.as_str().map(str::to_string))
            .collect();
        assert_eq!(names.len(), PRELOAD_NATIVE_TOOLS.len());
        for tool in PRELOAD_NATIVE_TOOLS {
            assert!(
                names.contains(&tool.to_string()),
                "missing preload tool {tool}"
            );
        }
        assert!(!names.contains(&"store_get_latest_digest".to_string()));
        assert!(!names.contains(&"pr_get_overview".to_string()));
        assert!(!names.contains(&"ci_rerun_workflow".to_string()));
    }

    #[test]
    fn is_lazy_native_tool_classification() {
        assert!(is_lazy_native_tool("tool_search"));
        assert!(is_lazy_native_tool("skill_load"));
        assert!(is_lazy_native_tool("read_file"));
        assert!(is_lazy_native_tool("grep"));
        assert!(is_lazy_native_tool("glob"));
        assert!(is_lazy_native_tool("bash_run"));
        assert!(is_lazy_native_tool("python_run"));
        assert!(is_lazy_native_tool("edit_file"));
        assert!(is_lazy_native_tool("write_file"));
        assert!(is_lazy_native_tool("web_fetch"));
        assert!(!is_lazy_native_tool("tool_describe"));
        assert!(!is_lazy_native_tool("pr_post_comment"));
        assert!(!is_lazy_native_tool("pr_get_overview"));
        assert!(!is_lazy_native_tool("ci_get_failure_digest"));
    }

    #[test]
    fn suggest_digest_name() {
        assert_eq!(
            ToolCatalog::new().suggest_tool_name("ci_get_failure_digest"),
            Some("ci_get_failure_digest".to_string())
        );
    }

    #[test]
    fn suggest_compound_overview_still_works() {
        assert_eq!(
            ToolCatalog::new().suggest_tool_name("pr_get_overview_and_changed_files"),
            Some("pr_get_overview".to_string())
        );
    }

    /// CI: covered by `cargo test` (no separate workflow step needed).
    ///
    /// Compares documented tool names in `skills/_base/TOOLS.md` against the static
    /// `TOOLS` catalog. Tools documented in TOOLS.md but missing from the catalog
    /// fail the test; catalog-only tools emit a warning so new harness tools can land
    /// before docs catch up.
    #[test]
    fn tools_md_matches_catalog() {
        let catalog: HashSet<String> = list_github_tool_names().into_iter().collect();
        let tools_md_path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("skills/_base/TOOLS.md");
        let tools_md =
            std::fs::read_to_string(&tools_md_path).unwrap_or_else(|e| {
                panic!("read {}: {e}", tools_md_path.display())
            });
        let documented = parse_tools_md_documented_names(&tools_md);

        let mut missing_from_md: Vec<String> = catalog
            .iter()
            .filter(|name| !documented.contains(*name))
            .cloned()
            .collect();
        missing_from_md.sort();
        if !missing_from_md.is_empty() {
            eprintln!(
                "WARN: tools in catalog but not documented in TOOLS.md: {}",
                missing_from_md.join(", ")
            );
        }

        let mut missing_from_catalog: Vec<String> = documented
            .iter()
            .filter(|name| !catalog.contains(*name))
            .cloned()
            .collect();
        missing_from_catalog.sort();
        assert!(
            missing_from_catalog.is_empty(),
            "TOOLS.md documents tools missing from catalog: {}",
            missing_from_catalog.join(", ")
        );
    }

    /// Extract documented tool names from TOOLS.md via lightweight regex (not a full MD parser).
    fn parse_tools_md_documented_names(content: &str) -> HashSet<String> {
        use regex::Regex;

        let mut names = HashSet::new();

        // `### `pr_get_overview`` and `### `read_file` / `grep` / `glob``
        let heading = Regex::new(r"(?m)^### (.+)$").expect("heading regex");
        let backtick = Regex::new(r"`([a-z][a-z0-9_]+)`").expect("backtick regex");
        for cap in heading.captures_iter(content) {
            for tool in backtick.captures_iter(&cap[1]) {
                names.insert(tool[1].to_string());
            }
        }

        // Lazy meta-tools table (no per-tool ### headings).
        if let Some(start) = content.find("## Lazy meta-tools") {
            let rest = &content[start..];
            let end = rest[1..]
                .find("\n## ")
                .map(|i| i + 1)
                .unwrap_or(rest.len());
            let section = &rest[..end];
            let table_row = Regex::new(r"(?m)^\| `([a-z][a-z0-9_]+)` \|").expect("table regex");
            for cap in table_row.captures_iter(section) {
                names.insert(cap[1].to_string());
            }
        }

        names
    }
}
