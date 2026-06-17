//! Chat harness tool whitelist, JSON contract hints, and fuzzy name suggestions.
//!
//! Runtime view merges static [`TOOLS`] specs with `config.chat.preferred_tools`.

use std::borrow::Cow;
use std::collections::HashSet;

use serde_json::Value;

#[derive(Debug, Clone, Copy)]
pub struct ToolSpec {
    pub name: &'static str,
    pub blurb: &'static str,
    pub required: &'static [&'static str],
    pub optional: &'static [&'static str],
}

const META_TOOLS: &[&str] = &["tool_list", "tool_describe", "tool_call"];
const MUTATING_TOOLS: &[&str] = &["ci_rerun_workflow", "pr_create_backport", "pr_post_comment"];

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
        blurb: "Capped unified diff",
        required: &["repo", "pr_number"],
        optional: &["max_bytes"],
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
        name: "pr_get_status",
        blurb: "Compact mergeability snapshot",
        required: &["repo", "pr_number"],
        optional: &[],
    },
    ToolSpec {
        name: "pr_list_merged",
        blurb: "Recently merged PRs",
        required: &["repo"],
        optional: &["since", "limit"],
    },
    ToolSpec {
        name: "pr_list_stale",
        blurb: "Stale open PRs",
        required: &["repo"],
        optional: &["days", "limit"],
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
        blurb: "Distilled failure logs",
        required: &["repo", "run_id"],
        optional: &["offset_lines", "max_lines"],
    },
    ToolSpec {
        name: "ci_list_runs",
        blurb: "List workflow runs for a branch",
        required: &["repo"],
        optional: &["branch", "limit"],
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
        name: "alert_list_open",
        blurb: "Dependabot / security alerts",
        required: &["repo"],
        optional: &["limit"],
    },
    ToolSpec {
        name: "store_get_latest_digest",
        blurb: "Latest local digest + approvals",
        required: &[],
        optional: &[],
    },
    ToolSpec {
        name: "tool_list",
        blurb: "Lazy MCP: list remote tools",
        required: &[],
        optional: &[],
    },
    ToolSpec {
        name: "tool_describe",
        blurb: "Lazy MCP: describe one tool",
        required: &["name"],
        optional: &[],
    },
    ToolSpec {
        name: "tool_call",
        blurb: "Lazy MCP: call by name",
        required: &["name", "args"],
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
        name: "pr_post_comment",
        blurb: "Mutating — harness queues approval on call",
        required: &["repo", "pr_number", "body"],
        optional: &[],
    },
];

/// Session-aware catalog: merges static specs with `config.chat.preferred_tools`.
#[derive(Debug, Clone, Copy)]
pub struct ToolCatalog<'a> {
    preferred: &'a [String],
}

impl ToolCatalog<'static> {
    /// Full static catalog (`preferred_tools` empty → all [`TOOLS`] whitelisted).
    #[cfg(test)]
    pub fn full() -> Self {
        Self { preferred: &[] }
    }
}

impl<'a> ToolCatalog<'a> {
    pub fn new(preferred: &'a [String]) -> Self {
        Self { preferred }
    }

    /// Meta + mutating always allowed; otherwise preferred list or full static catalog.
    pub fn is_known_chat_tool(&self, name: &str) -> bool {
        if META_TOOLS.contains(&name) || MUTATING_TOOLS.contains(&name) {
            return true;
        }
        if self.preferred.is_empty() {
            return spec_by_name(name).is_some();
        }
        self.preferred.iter().any(|t| t == name)
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
        let preferred_set = self.preferred_set();
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
            if preferred_set.contains(name.as_str()) {
                score += 100;
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
            "Invalid tool_name `{bad}`. Call one tool at a time — do not invent combined names."
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
        out.push_str(&self.preferred_summary_line());
        out
    }

    pub fn format_unknown_tool_nudge(&self, bad: &str) -> String {
        let fallback = self
            .preferred
            .first()
            .map(String::as_str)
            .or_else(|| TOOLS.first().map(|t| t.name))
            .unwrap_or("pr_list_open");
        let suggestion = self.suggest_tool_name(bad).unwrap_or_else(|| fallback.to_string());
        format!(
            "Unknown tool_name `{bad}`. Did you mean `{suggestion}`? ({}){}{}",
            self.tool_blurb(&suggestion),
            self.format_tool_contract_block(&suggestion, None, None, None),
            self.preferred_summary_line()
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
        out.push_str(&self.format_tool_contract_block(
            tool_name,
            pr,
            run_id,
            example_repo,
        ));
        out.push_str(&self.preferred_summary_line());
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
                        .or_else(|| {
                            v.as_i64()
                                .filter(|n| *n >= 0)
                                .map(|n| n as u64)
                        })
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

    /// After a failed MCP call: surface the error, args sent, and follow-up guidance.
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
        let sent = serde_json::to_string_pretty(tool_args).unwrap_or_else(|_| tool_args.to_string());
        let err = crate::agent::context::truncate_chars(error_body, 1200);
        let mut out = format!(
            "Tool `{tool_name}` failed.\n\nError:\n{err}\n\nArgs sent:\n{sent}"
        );
        out.push_str(&self.format_required_optional(tool_name));
        if self.missing_required_fields(tool_name, tool_args).is_empty() {
            out.push_str(&self.format_actionable_failure_followup(
                tool_name,
                tool_args,
                error_body,
                configured_repos,
            ));
        } else {
            out.push_str(&self.format_tool_contract_block(tool_name, pr, run_id, repo));
        }
        out.push_str(&self.preferred_summary_line());
        out
    }

    fn format_actionable_failure_followup(
        &self,
        tool_name: &str,
        tool_args: &Value,
        error_body: &str,
        configured_repos: &[String],
    ) -> String {
        let low = error_body.to_ascii_lowercase();
        let repo = tool_args
            .get("repo")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        let mut out = String::from(
            "\n\nArgs include all required fields — this is not a missing-parameter error.",
        );
        if low.contains("not found")
            || low.contains("http 404")
            || low.contains("could not resolve to a repository")
        {
            out.push_str(
                "\nGitHub could not find the repo, PR, workflow run, or log for those IDs.",
            );
            if !configured_repos.is_empty() {
                out.push_str("\nConfigured repos: ");
                out.push_str(&configured_repos.join(", "));
                if !repo.is_empty() && !configured_repos.iter().any(|r| r.as_str() == repo) {
                    out.push_str(&format!(
                        "\n`{repo}` is not in that list — pick a configured repo or update coworker.yaml."
                    ));
                }
            }
            if matches!(tool_name, "ci_get_failed_logs" | "ci_get_run_summary") {
                out.push_str(
                    "\nConfirm `run_id` came from `ci_analyze_pr_failures` or `ci_get_run_summary` for the same `repo`.",
                );
            }
            if tool_name == "ci_get_failed_logs" && low.contains("log not found") {
                out.push_str(
                    "\nLogs may be pending or expired — try `ci_get_run_summary` before fetching failed logs.",
                );
            }
        } else if low.contains("temporary server error")
            || low.contains("http 504")
            || low.contains("http 503")
            || low.contains("http 502")
            || low.contains("gateway timeout")
        {
            out.push_str("\nTransient GitHub error — retry the same call once.");
        } else if low.contains("permission")
            || low.contains("http 403")
            || low.contains("forbidden")
        {
            out.push_str("\nPermission denied — check GH_TOKEN / `gh auth login` for this repo.");
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
             One tool per turn. Mutating tools are queued for approval automatically."
        )
    }

    fn format_generic_contract(&self) -> String {
        let example_tool = self
            .preferred
            .first()
            .map(String::as_str)
            .unwrap_or("pr_list_open");
        format!(
            "Call `{example_tool}` via the native tool API with arguments like:\n{}\n\
             Then one tool per turn for each follow-up (e.g. pr_get_overview with pr_number).",
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
            .unwrap_or(Cow::Borrowed(
                "configured preferred tool — see TOOLS.md or tool_describe",
            ))
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
            "pr_list_changed_files" => &["pr_get_diff", "pr_get_overview"],
            "ci_analyze_pr_failures" => &["ci_get_run_summary", "ci_get_failed_logs"],
            "pr_list_open" => &["pr_get_overview", "pr_list_waiting_review"],
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

    fn preferred_summary_line(&self) -> String {
        if self.preferred.is_empty() {
            return String::new();
        }
        format!(
            "\n\nSession preferred_tools: {}",
            self.preferred
                .iter()
                .map(|t| format!("`{t}`"))
                .collect::<Vec<_>>()
                .join(", ")
        )
    }

    fn preferred_set(&self) -> HashSet<&str> {
        self.preferred.iter().map(String::as_str).collect()
    }

    fn candidate_names(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let mut seen = HashSet::new();
        for t in self.preferred {
            if seen.insert(t.as_str()) {
                out.push(t.clone());
            }
        }
        for spec in TOOLS {
            if seen.insert(spec.name) {
                out.push(spec.name.to_string());
            }
        }
        out
    }

    fn merged_prefixes(&self) -> Vec<String> {
        let mut names = self.candidate_names();
        names.sort_by_key(|p| std::cmp::Reverse(p.len()));
        names
    }

    /// OpenAI/Ollama-native `tools` array for the chat LLM API.
    pub fn native_tool_definitions(&self) -> Vec<Value> {
        self.candidate_names()
            .into_iter()
            .filter(|name| self.is_known_chat_tool(name))
            .filter_map(|name| spec_by_name(&name).map(native_tool_from_spec))
            .collect()
    }
}

fn json_type_for_arg(key: &str) -> &'static str {
    match key {
        "repo" | "author" | "branch" | "body" | "target_branch" | "since" | "name" => "string",
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
    if matches!(name, "store_get_latest_digest" | "tool_list") {
        return (&[], &[]);
    }
    if matches!(name, "tool_describe") {
        return (&["name"], &[]);
    }
    if matches!(name, "tool_call") {
        return (&["name", "args"], &[]);
    }
    if name == "ci_get_failed_logs" {
        return (&["repo", "run_id"], &["offset_lines", "max_lines"]);
    }
    if name == "ci_get_run_summary" {
        return (&["repo", "run_id"], &[]);
    }
    if name == "ci_list_runs" {
        return (&["repo"], &["branch", "limit"]);
    }
    if name == "pr_list_open" {
        return (&["repo"], &["author", "limit"]);
    }
    if name == "pr_list_waiting_review" {
        return (&["repo"], &["limit"]);
    }
    if name == "pr_list_stale" {
        return (&["repo"], &["days", "limit"]);
    }
    if name == "pr_list_merged" {
        return (&["repo"], &["since", "limit"]);
    }
    if name == "pr_get_diff" {
        return (&["repo", "pr_number"], &["max_bytes"]);
    }
    if name == "issue_list_open" || name == "alert_list_open" {
        return (&["repo"], &["limit"]);
    }
    if name == "issue_get" {
        return (&["repo", "issue_number"], &[]);
    }
    if name == "issue_add_label" {
        return (&["repo", "issue_number", "label"], &[]);
    }
    if name.starts_with("pr_list_") || name == "ci_list_runs" {
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
        | "ci_analyze_pr_failures" => format!(
            "{{\"repo\":\"{repo}\",\"pr_number\":{pr}}}"
        ),
        "pr_get_diff" => format!(
            "{{\"repo\":\"{repo}\",\"pr_number\":{pr},\"max_bytes\":48000}}"
        ),
        "pr_list_open" => format!(
            r#"{{"repo":"{repo}","author":"@me","limit":20}}"#
        ),
        "pr_list_waiting_review"
        | "pr_list_merged"
        | "pr_list_stale"
        | "issue_list_open"
        | "alert_list_open"
        | "ci_list_runs" => format!("{{\"repo\":\"{repo}\"}}"),
        "ci_get_run_summary" | "ci_get_failed_logs" => {
            format!("{{\"repo\":\"{repo}\",\"run_id\":{run}}}")
        }
        "issue_get" => format!(r#"{{"repo":"{repo}","issue_number":42}}"#),
        "store_get_latest_digest" | "tool_list" => "{}".to_string(),
        "tool_describe" => r#"{"name":"pr_get_overview"}"#.to_string(),
        "tool_call" => format!(r#"{{"name":"pr_list_open","args":{{"repo":"{repo}"}}}}"#),
        "ci_rerun_workflow" => format!("{{\"repo\":\"{repo}\",\"run_id\":{run}}}"),
        "pr_post_comment" => {
            format!("{{\"repo\":\"{repo}\",\"pr_number\":{pr},\"body\":\"…\"}}")
        }
        "pr_create_backport" => format!(
            "{{\"repo\":\"{repo}\",\"pr_number\":{pr},\"target_branch\":\"release/3.x\"}}"
        ),
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
    format!(
        "{{\"action\":\"tool\",\"tool_name\":\"{tool_name}\",\"tool_args\":{args}}}"
    )
}

fn common_prefix_len(a: &str, b: &str) -> usize {
    a.chars()
        .zip(b.chars())
        .take_while(|(x, y)| x == y)
        .count()
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
    use crate::config::default_chat_preferred_tools;

    #[test]
    fn suggest_compound_overview_name() {
        assert_eq!(
            ToolCatalog::full()
                .suggest_tool_name("pr_get_overview_and_changed_files_combined_for_prs_19264"),
            Some("pr_get_overview".to_string())
        );
    }

    #[test]
    fn salvage_extracts_pr_from_compound_name() {
        let (name, pr) = ToolCatalog::full()
            .salvage_hallucinated_tool_name(
                "pr_get_overview_and_changed_files_combined_for_prs_19264_19263",
            )
            .unwrap();
        assert_eq!(name, "pr_get_overview");
        assert_eq!(pr, Some(19264));
    }

    #[test]
    fn invalid_nudge_includes_suggestion_and_native_args() {
        let msg =
            ToolCatalog::full().format_invalid_tool_nudge("pr_get_overview_and_changed_files_combined");
        assert!(msg.contains("Did you mean `pr_get_overview`"));
        assert!(msg.contains("Call `pr_get_overview`"));
        assert!(msg.contains("pr_number"));
    }

    #[test]
    fn args_nudge_uses_concrete_repo_in_example_json() {
        let msg = ToolCatalog::full().format_tool_args_nudge(
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
        let msg = ToolCatalog::full().format_tool_args_nudge("pr_get_overview", "pr_number", None, None);
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
        let msg = ToolCatalog::full().format_tool_failure_nudge(
            "ci_get_failed_logs",
            &args,
            err,
            &["acme/widget".into()],
        );
        assert!(msg.contains("not a missing-parameter error"));
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
        let msg = ToolCatalog::full().format_tool_failure_nudge(
            "ci_get_failed_logs",
            &args,
            err,
            &["acme/widget".into()],
        );
        assert!(msg.contains("not in that list"));
        assert!(!msg.contains("is missing required"));
    }

    #[test]
    fn failure_nudge_includes_error_args_and_schema() {
        let args = serde_json::json!({ "repo": "acme/widget" });
        let msg = ToolCatalog::full().format_tool_failure_nudge(
            "pr_get_overview",
            &args,
            "HTTP 404: Not Found",
            &["acme/widget".into()],
        );
        assert!(msg.contains("HTTP 404"));
        assert!(msg.contains("acme/widget"));
        assert!(msg.contains("Call `pr_get_overview`"));
    }

    #[test]
    fn known_tool_whitelist_matches_tools_md() {
        let cat = ToolCatalog::full();
        assert!(cat.is_known_chat_tool("pr_get_overview"));
        assert!(cat.is_known_chat_tool("store_get_latest_digest"));
        assert!(!cat.is_known_chat_tool("pr_get_overview_and_friends"));
    }

    #[test]
    fn custom_preferred_whitelist() {
        let preferred = vec!["pr_list_stale".into(), "pr_get_overview".into()];
        let cat = ToolCatalog::new(&preferred);
        assert!(cat.is_known_chat_tool("pr_list_stale"));
        assert!(cat.is_known_chat_tool("tool_list"));
        assert!(!cat.is_known_chat_tool("pr_list_merged"));
    }

    #[test]
    fn preferred_only_tool_gets_inferred_args_nudge() {
        let preferred = vec!["pr_list_stale".into()];
        let cat = ToolCatalog::new(&preferred);
        let msg = cat.format_tool_args_nudge("pr_list_stale", "repo", None, None);
        assert!(msg.contains("Required tool_args: repo"));
        assert!(msg.contains("Session preferred_tools: `pr_list_stale`"));
    }

    #[test]
    fn nudge_lists_session_preferred_tools() {
        let preferred = default_chat_preferred_tools();
        let cat = ToolCatalog::new(&preferred);
        let msg = cat.format_invalid_tool_nudge("not_a_real_tool_xyz");
        assert!(msg.contains("Session preferred_tools:"));
        assert!(msg.contains("`pr_get_overview`"));
    }

    #[test]
    fn pr_list_open_nudge_lists_optional_filters() {
        let msg = ToolCatalog::full().format_tool_args_nudge("pr_list_open", "repo", None, None);
        assert!(msg.contains("Required tool_args: repo"));
        assert!(msg.contains("author"));
        assert!(msg.contains("limit"));
    }

    #[test]
    fn preferred_only_stale_lists_optional_days() {
        let preferred = vec!["pr_list_stale".into()];
        let cat = ToolCatalog::new(&preferred);
        let msg = cat.format_tool_args_nudge("pr_list_stale", "repo", None, None);
        assert!(msg.contains("days"));
        assert!(msg.contains("limit"));
    }

    #[test]
    fn failed_logs_lists_paging_optional() {
        let msg = ToolCatalog::full().format_tool_args_nudge("ci_get_failed_logs", "run_id", None, None);
        assert!(msg.contains("offset_lines"));
        assert!(msg.contains("max_lines"));
    }

    #[test]
    fn empty_preferred_uses_full_catalog() {
        let cat = ToolCatalog::new(&[]);
        assert!(cat.is_known_chat_tool("pr_list_stale"));
        assert!(cat.is_known_chat_tool("pr_list_merged"));
    }

    #[test]
    fn suggest_boosts_preferred_tool() {
        let preferred = vec!["pr_get_overview".into(), "pr_list_open".into()];
        let cat = ToolCatalog::new(&preferred);
        assert_eq!(
            cat.suggest_tool_name("pr_get_overview_and_changed_files"),
            Some("pr_get_overview".to_string())
        );
    }
}
