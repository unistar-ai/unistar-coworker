# Base tools

Shared MCP + harness tool vocabulary for chat and workflows.  
Mutating tools require **approval** in chat — use `action:approval`, never `action:tool`.

All GitHub tools take `repo` as `owner/repo` (e.g. `acme/widget`).

---

## Read-only MCP (unistar-mcp)

### `pr_get_overview`
Single-call PR snapshot: status, CI/review summary, changed-file **counts**, failing run IDs.

| Param | Required | Notes |
|-------|----------|-------|
| `repo` | yes | Repository |
| `pr_number` | yes | Pull request number |

### `pr_get_status`
Compact mergeability snapshot (CI, review, draft, mergeable). Fallback when overview is unavailable.

| Param | Required | Notes |
|-------|----------|-------|
| `repo` | yes | |
| `pr_number` | yes | |

### `pr_get_merge_blockers`
Structured merge blockers: conflicts, checks, review state, draft.

| Param | Required | Notes |
|-------|----------|-------|
| `repo` | yes | |
| `pr_number` | yes | |

### `pr_get_review_state`
Review requests, latest reviews, inline comment snippets. **In default chat whitelist.** After `pr_get_merge_blockers`.

| Param | Required | Notes |
|-------|----------|-------|
| `repo` | yes | |
| `pr_number` | yes | |

### `pr_get_review_routing`
CODEOWNERS-based reviewers for changed files. Next: `pr_get_review_state`.

| Param | Required | Notes |
|-------|----------|-------|
| `repo` | yes | |
| `pr_number` | yes | |

### `pr_list_changed_files`
Changed file paths with `+`/`-` line counts (not full patch).

| Param | Required | Notes |
|-------|----------|-------|
| `repo` | yes | |
| `pr_number` | yes | |

### `pr_diff_risk_scan`
Heuristic risk flags: lockfile, migration, workflow edits, large diffs. **In default chat whitelist.** After `pr_list_changed_files`.

| Param | Required | Notes |
|-------|----------|-------|
| `repo` | yes | |
| `pr_number` | yes | |

### `pr_get_diff`
Unified diff. On **large PRs**, prefer `pr_list_changed_files` then fetch **one file at a time** with `path`.

| Param | Required | Notes |
|-------|----------|-------|
| `repo` | yes | |
| `pr_number` | yes | |
| `path` | no | Single changed-file path (exact match from `pr_list_changed_files`) |
| `max_bytes` | no | Max diff bytes returned (default **48000** full PR, **64000** with `path`) |

### `repo_get_info`
Repository metadata: default branch, visibility, language, license, topics, label names.

| Param | Required | Notes |
|-------|----------|-------|
| `repo` | yes | |
| `label_limit` | no | Max labels listed (default **20**, max **50**) |

### `pr_list_open`
Open PRs, newest first, one compact CI/review line each.

| Param | Required | Notes |
|-------|----------|-------|
| `repo` | yes | |
| `author` | no | Filter: `"@me"` for your PRs, or a GitHub login. Omit = all authors |
| `limit` | no | Max PRs (default **20**) |

### `pr_list_waiting_review`
Open PRs with **passing CI** that still need review (not draft).

| Param | Required | Notes |
|-------|----------|-------|
| `repo` | yes | |
| `limit` | no | Max PRs (default **20**) |

### `pr_list_merge_ready`
Open PRs that are **merge-ready**: CI green, approved, mergeable (not draft).

| Param | Required | Notes |
|-------|----------|-------|
| `repo` | yes | |
| `limit` | no | Max open PRs scanned (default **30**, max **50**) |

### `pr_list_merge_blocked`
Open PRs with **green CI** but not merge-ready (draft, conflicts, review pending, etc.).

| Param | Required | Notes |
|-------|----------|-------|
| `repo` | yes | |
| `limit` | no | Max open PRs scanned (default **30**, max **50**) |

Next: `pr_get_merge_blockers` on top rows.

### `pr_list_stale`
Open PRs with no updates for at least N days.

| Param | Required | Notes |
|-------|----------|-------|
| `repo` | yes | |
| `days` | no | Minimum idle days (default **7**) |
| `limit` | no | Max PRs (default **20**) |

### `pr_list_merged`
Recently merged PRs (release notes / regression link / release-duty backport label scan).

| Param | Required | Notes |
|-------|----------|-------|
| `repo` | yes | |
| `since` | no | ISO date `YYYY-MM-DD` or days-ago number string (default **14**) |
| `label` | no | Filter to PRs with this label (e.g. backport label) |
| `limit` | no | Max PRs (default **30**) |

### `pr_get_status`
Compact mergeability snapshot for one PR.

| Param | Required | Notes |
|-------|----------|-------|
| `repo` | yes | |
| `pr_number` | yes | |

### `pr_get_status_batch`
Batch CI/review lines for multiple PRs in **one GraphQL call** (max 15). Same format as `pr_list_open`.

| Param | Required | Notes |
|-------|----------|-------|
| `repo` | yes | |
| `pr_numbers` | yes | Comma-separated, e.g. `"42,43,99"` |

Use after `pr_list_waiting_review` instead of N × `pr_get_status`.

### `pr_get_overview_batch`
Lightweight multi-PR overview in **one GraphQL call** (max **5**): CI counts, review, file stats. **No failing run IDs** — follow with `pr_get_overview` or `ci_analyze_pr_failures` on red PRs.

| Param | Required | Notes |
|-------|----------|-------|
| `repo` | yes | |
| `pr_numbers` | yes | Comma-separated, e.g. `"42,43"` |

### `pr_list_backport_candidates`
Merged PRs with backport label (default `needs-backport`). Release-duty.

| Param | Required | Notes |
|-------|----------|-------|
| `repo` | yes | |
| `label` | no | Default **needs-backport** |
| `since` | no | Default **14** days |
| `limit` | no | Default **30** |

### `pr_is_docs_only`
Whether a PR changes only docs paths (scheduler skip hint).

| Param | Required |
|-------|----------|
| `repo` | yes |
| `pr_number` | yes |

### `pr_list_large`
Open PRs exceeding file or line thresholds (mega-PR hygiene). Next: `pr_diff_risk_scan`.

| Param | Required | Notes |
|-------|----------|-------|
| `repo` | yes | |
| `min_files` | no | Default **30** |
| `min_lines` | no | Default **1000** (additions + deletions) |
| `limit` | no | Max open PRs scanned (default **40**) |

### `pr_get_ci_snapshot`
One-call PR CI triage: **`CI_KIND`** + failing run list + compact **`ci_get_failure_digest`** per run (default **2**, max **5**). Includes **Flaky hint** from webhook ledger when configured.

| Param | Required | Notes |
|-------|----------|-------|
| `repo` | yes | |
| `pr_number` | yes | |
| `max_runs` | no | Digests to include (default **2**) |

Resource: `github://pull/{owner}/{repo}/{number}/ci-snapshot`

### `ci_analyze_pr_failures`
Failing workflow run IDs for a PR. **First line:** `CI_KIND: actions_only | external_only | mixed | pending | approval | clean` — route before calling log tools.

| Param | Required | Notes |
|-------|----------|-------|
| `repo` | yes | |
| `pr_number` | yes | |

When `CI_KIND` is `external_only` or `pending`, do **not** call `ci_get_failed_logs`.

### `ci_get_run_summary`
Run status, conclusion, duration, failed job names. **Call before** full logs.

| Param | Required | Notes |
|-------|----------|-------|
| `repo` | yes | |
| `run_id` | yes | From `ci_analyze_pr_failures` |

### `ci_get_failed_logs`
Distilled failure logs with **synopsis** (failed job/step, test name, error sig, FP) plus error excerpts. Prefer passing `job_id` from `ci_get_run_summary` on matrix workflows.

| Param | Required | Notes |
|-------|----------|-------|
| `repo` | yes | |
| `run_id` | yes | |
| `job_id` | no | Failed job ID from `ci_get_run_summary` (recommended for matrix) |
| `focus` | no | `last` (default), `all`, or `step:<name>` from run summary |
| `offset_lines` | no | Line offset for paging (default **0**; use `next_offset_lines` from prior page) |
| `max_lines` | no | Lines per page (default **0** = single chunk; set e.g. **80** to page) |

Next: `policy_classify_failure` or `ci_get_failure_digest` for a lighter snapshot.

### `ci_get_failure_digest`
Compact failure digest: synopsis + policy verdict + ~1KB log excerpt. Use before paging full logs.

| Param | Required | Notes |
|-------|----------|-------|
| `repo` | yes | |
| `run_id` | yes | |
| `job_id` | no | Optional failed job scope |

### `ci_get_job_logs`
Distilled logs for **one workflow job** (`job_id` from `ci_get_run_summary`). Use when `ci_get_failed_logs` is too large.

| Param | Required | Notes |
|-------|----------|-------|
| `repo` | yes | |
| `run_id` | yes | |
| `job_id` | yes | From run summary |
| `offset_lines` | no | Paging (same as `ci_get_failed_logs`) |
| `max_lines` | no | Paging |

### `ci_list_runs`
Recent Actions runs on a branch (main-guard, CI reports). **In default chat whitelist.**

| Param | Required | Notes |
|-------|----------|-------|
| `repo` | yes | |
| `branch` | no | Branch name (default: repo default branch) |
| `limit` | no | Max runs (default **15**, max **50**) |

### `ci_branch_health`
Branch CI rollup: failure rate, streak, last failing run. **In default chat whitelist.** Prefer over reading many `ci_list_runs` lines.

| Param | Required | Notes |
|-------|----------|-------|
| `repo` | yes | |
| `branch` | no | Default branch when omitted |
| `limit` | no | Runs to analyze (default **15**) |

### `ci_workflow_stats`
Per-workflow CI rollup on a branch: run count, failure rate, avg/max duration. Find noisy workflows.

| Param | Required | Notes |
|-------|----------|-------|
| `repo` | yes | |
| `branch` | no | Default branch when omitted |
| `limit` | no | Runs sampled (default **30**) |
| `top` | no | Workflows listed (default **10**, max **20**) |

Next: `ci_branch_health` for streak; `ci_get_run_summary` on failing workflows.

### `ci_list_workflows`
GitHub Actions workflow names and IDs for the repo.

| Param | Required | Notes |
|-------|----------|-------|
| `repo` | yes | |
| `limit` | no | Default **30**, max **100** |

### `ci_correlate_prs`
Recently merged PRs on the run branch **before** a failing run (regression-link).

| Param | Required | Notes |
|-------|----------|-------|
| `repo` | yes | |
| `run_id` | yes | Failing run |
| `window_days` | no | Default **7** |
| `limit` | no | Default **10** |

### `ci_failure_fingerprint`
Structured failure fingerprint (job, step, test, error signature). Aligns with flaky ledger `compute_fingerprint`.

| Param | Required | Notes |
|-------|----------|-------|
| `repo` | yes | |
| `run_id` | yes | From `ci_analyze_pr_failures` |

### `policy_classify_failure`
Rule-based failure class: `test` / `infra` / `auth` / `timeout` / `external_ci`. **In default chat whitelist.** Call after `ci_failure_fingerprint`.

| Param | Required | Notes |
|-------|----------|-------|
| `repo` | yes | |
| `run_id` | yes | |

### `pr_draft_ci_comment`
Draft markdown PR comment for a CI failure (policy verdict + fingerprint). Read-only — edit, then `pr_post_comment` (approval).

| Param | Required | Notes |
|-------|----------|-------|
| `repo` | yes | |
| `pr_number` | yes | |
| `run_id` | yes | From `ci_analyze_pr_failures` |

### `ci_compare_runs`
Compare two runs by fingerprint without full logs — use after rerun.

| Param | Required | Notes |
|-------|----------|-------|
| `repo` | yes | |
| `run_id_a` | yes | Often older/base run |
| `run_id_b` | yes | Often newer/compare run |

### `ci_list_external_checks`
External (non-GitHub Actions) status checks on a PR — Jenkins, Codecov, etc.

| Param | Required | Notes |
|-------|----------|-------|
| `repo` | yes | |
| `pr_number` | yes | |

Do **not** call `ci_get_failed_logs` for these checks.

### `ci_get_check_url`
External check names with details URLs (open in browser). Pair with `ci_list_external_checks`.

| Param | Required | Notes |
|-------|----------|-------|
| `repo` | yes | |
| `pr_number` | yes | |

### `issue_list_open`
Open issues with compact title/label summary.

| Param | Required | Notes |
|-------|----------|-------|
| `repo` | yes | |
| `limit` | no | Max issues (default **20**) |

### `issue_get`
Single issue title, body (capped), labels, state.

| Param | Required | Notes |
|-------|----------|-------|
| `repo` | yes | |
| `issue_number` | yes | |

### `issue_search`
Search issues with GitHub query syntax.

| Param | Required | Notes |
|-------|----------|-------|
| `repo` | yes | |
| `query` | yes | GitHub search string |
| `limit` | no | Default **20**, max **50** |

### `release_list_tags`
Recent git tags (newest first).

| Param | Required | Notes |
|-------|----------|-------|
| `repo` | yes | |
| `limit` | no | Default **20**, max **50** |

### `release_notes_draft`
Release-notes bullets from PRs merged since a tag.

| Param | Required | Notes |
|-------|----------|-------|
| `repo` | yes | |
| `since_tag` | no | Default latest tag |
| `limit` | no | Default **30** |

### `alert_list_open`
Open Dependabot security alerts (severity + summary).

| Param | Required | Notes |
|-------|----------|-------|
| `repo` | yes | |
| `limit` | no | Max alerts (default **20**) |

### `alert_summarize_open`
Dependabot severity rollup (counts + top summaries). Next: `alert_list_open` for full list.

| Param | Required | Notes |
|-------|----------|-------|
| `repo` | yes | |
| `limit` | no | Max alerts scanned (default **100**) |

---

## Lazy meta-tools (MCP + chat harness)

When MCP runs in `--lazy` mode:

| Tool | Required `tool_args` | Optional |
|------|----------------------|----------|
| `skill_load` | `name` | Skill from **Available skills** in system prompt; warms `tools[]` schemas |
| `tool_search` | `query` | `limit` |
| `tool_list_category` | `category` | CI, PR, Repo, … |
| `tool_list` | (none) | cached per chat session |
| `tool_describe` | `name` | optional if using `tool_call` |
| `tool_call` | `name`, `args` | |
| `resource_read` | `uri` | harness-only; e.g. `github://pull/.../ci-snapshot` |

---

## Harness (local Store — no GitHub API)

### `store_list_pending_approvals`
Pending mutating-action queue.

| Param | Required | Notes |
|-------|----------|-------|
| `limit` | no | Max rows (default **20**, max **50**) |

---

## Coding harness (local workspace)

Chat tools split into two execution paths:

| Path | Tools | Behavior |
|------|-------|----------|
| **No approval** (read-only scheduling) | `read_file`, `grep`, `glob`, `web_fetch`, **`bash_run`**, **`python_run`**, **`edit_file`**, **`write_file`**, most MCP reads | May run **in parallel**; review-gated tools pass **LLM safety review** first — on REJECT they fall back to **human approval** (not an immediate error) |
| **Approval required** | `ci_rerun_workflow`, `pr_post_comment`, `pr_create_backport`, `issue_add_label` | **At most one per turn**; user confirms in Approvals UI |

### `read_file` / `grep` / `glob`
Read-only file tools under `chat.workspace`. See chat agent for usage.

### `bash_run`
Run a **shell command or short script** in `chat.workspace`. **No-approval path** — built-in LLM safety review + static preflight; single-line via `sh -c`, multiline via `sh -s` stdin; may parallelize with other read-only tools.

| Param | Required | Notes |
|-------|----------|-------|
| `command` | yes | Single line (preferred for simple ops) or multiline script (max 200 lines) |
| `cwd` | no | Relative to workspace |

Config (`chat.bash`): `timeout_secs`, `max_output_chars`.

### `python_run`
Run a **multiline** Python snippet in `chat.workspace`. **No-approval path** — built-in LLM safety review + static preflight; code fed on stdin to `python3 -u -`.

| Param | Required | Notes |
|-------|----------|-------|
| `code` | yes | Multiline Python source |
| `cwd` | no | Relative to workspace (default: workspace root) |

Config (`chat.python`): `timeout_secs`, `max_output_chars`, `command` (default `python3`).

**Use for:** parsing/transforming data, quick probes, REPL-style checks. Prefer `bash_run` for shell pipelines.

### `web_fetch`
Fetch readable text from a URL or local HTML file under `chat.workspace`. Returns structured metadata plus body.

| Param | Required | Notes |
|-------|----------|-------|
| `url` | yes | `http(s)://…`, `localhost:PORT` (needs `allow_localhost`), or workspace HTML path |
| `mode` | no | `full` (default), `metadata` (title/headings/links only), `links` |
| `max_chars` | no | Body cap — default **32000** (`full`) or **8000** (`metadata`/`links`) |
| `browser` | no | `true` = headless Chromium (JS render / anti-bot); pass on the tool call when needed |

Config (`chat.web_fetch`): `timeout_secs`, `max_content_chars`, `allow_localhost`, `cache_ttl_secs`, `user_agent`, `browser_timeout_secs`, `browser_wait_ms`, `chromium_path`.

**Browser mode:** Use for JS-heavy pages or sites with anti-bot challenges (e.g. zhihu question URLs). Slower (~seconds/page); requires Chrome/Chromium installed.

**Not for:** GitHub PR/CI (use MCP), authenticated pages without user cookies.

### `edit_file` / `write_file`
Workspace file edits. **No-approval path** — built-in LLM safety review + static preflight; may parallelize with other read-only tools.

| Tool | Required | Notes |
|------|----------|-------|
| `edit_file` | `path`, `old_string`, `new_string` | Unique match unless `replace_all: true` |
| `write_file` | `path`, `content` | `create_only: true` refuses overwrite |

**Use for:** surgical patches (`edit_file`) or new files (`write_file`). Always `read_file` first to copy exact `old_string`.

---

## Mutating (approval only)

Use `action:approval` in chat. Workflows may call directly when configured.

### `ci_rerun_workflow`
Rerun **failed jobs** in a workflow run (after inspecting logs).

| Param | Required |
|-------|----------|
| `repo` | yes |
| `run_id` | yes |

### `pr_post_comment`
Post a PR comment.

| Param | Required |
|-------|----------|
| `repo` | yes |
| `pr_number` | yes |
| `body` | yes (markdown) |

### `pr_create_backport`
Cherry-pick a merged PR onto a target branch and open a backport PR.

| Param | Required |
|-------|----------|
| `repo` | yes |
| `pr_number` | yes (merged PR) |
| `target_branch` | yes (e.g. `release/3.15`) |

### `backport_get_conflict_files`
List unmerged files in a backport workspace after `pr_create_backport` conflict (read-only).

| Param | Required | Notes |
|-------|----------|-------|
| `workspace_path` | yes | From backport error message (`unistar-backport-*`) |

### `backport_suggest_resolution`
Conflict marker hints (ours vs theirs line counts) in backport workspace.

| Param | Required | Notes |
|-------|----------|-------|
| `workspace_path` | yes | From backport error message |
| `max_files` | no | Files to analyze (default **3**) |

### `issue_add_label`
Add a label to an issue (workflow / approval — not in default chat whitelist).

| Param | Required |
|-------|----------|
| `repo` | yes |
| `issue_number` | yes |
| `label` | yes |

### `notify_post_slack`
Post a compact Slack message via incoming webhook (mutating — not in default chat whitelist).

| Param | Required | Notes |
|-------|----------|-------|
| `text` | yes | Short summary; avoid raw logs |
| `webhook_url` | no | Else `SLACK_WEBHOOK_URL` on MCP server |

Use `config.output.slack_webhook` for automated Slack posts from coworker; this MCP tool is for agent-driven notifications.

### `event_list_recent`
List recent GitHub webhook events (read-only). Events persist to **`~/.cache/unistar-mcp/events.jsonl`** by default (`UNISTAR_MCP_EVENT_FILE`; set `off` for memory-only) so stdio coworker and HTTP webhook ingest share the buffer.

| Param | Required | Notes |
|-------|----------|-------|
| `repo` | no | Filter `owner/repo` |
| `kind` | no | Prefix filter, e.g. `pull_request` or `workflow_run` |
| `limit` | no | Default **20**, max **100** |

Failed `workflow_run` rows may include async **`fp:`** fingerprint. Point GitHub webhooks to `POST /hooks/github` on `unistar-mcp http`. On `pull_request.*`, chain to `pr_get_overview`.

## Tool chains (quick reference)

```
pr_get_overview → ci_analyze_pr_failures → ci_get_run_summary → ci_get_failure_digest → ci_get_failed_logs → policy_classify_failure → ci_rerun_workflow → ci_compare_runs
ci_list_workflows → ci_list_runs → ci_branch_health → ci_correlate_prs
pr_list_waiting_review → pr_get_status_batch | pr_get_overview_batch → pr_get_overview
release_list_tags → release_notes_draft
pr_list_backport_candidates → pr_create_backport → backport_get_conflict_files
```

Read cache: **`UNISTAR_MCP_CACHE_TTL`** (default **60s** for overview, branch health, repo info).
