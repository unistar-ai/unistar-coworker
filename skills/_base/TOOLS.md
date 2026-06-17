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

### `pr_list_changed_files`
Changed file paths with `+`/`-` line counts (not full patch).

| Param | Required | Notes |
|-------|----------|-------|
| `repo` | yes | |
| `pr_number` | yes | |

### `pr_get_diff`
Capped unified diff (harness compresses per-file). Use after `pr_list_changed_files` when patch detail is needed.

| Param | Required | Notes |
|-------|----------|-------|
| `repo` | yes | |
| `pr_number` | yes | |
| `max_bytes` | no | Max diff bytes returned (default **48000**) |

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

### `pr_list_stale`
Open PRs with no updates for at least N days.

| Param | Required | Notes |
|-------|----------|-------|
| `repo` | yes | |
| `days` | no | Minimum idle days (default **7**) |
| `limit` | no | Max PRs (default **20**) |

### `pr_list_merged`
Recently merged PRs (release notes / regression link).

| Param | Required | Notes |
|-------|----------|-------|
| `repo` | yes | |
| `since` | no | ISO date `YYYY-MM-DD` or days-ago number string (default **14**) |
| `limit` | no | Max PRs (default **30**) |

### `ci_analyze_pr_failures`
Failing workflow run IDs for a PR (input to `ci_get_run_summary` / `ci_get_failed_logs`).

| Param | Required | Notes |
|-------|----------|-------|
| `repo` | yes | |
| `pr_number` | yes | |

### `ci_get_run_summary`
Run status, conclusion, duration, failed job names. **Call before** full logs.

| Param | Required | Notes |
|-------|----------|-------|
| `repo` | yes | |
| `run_id` | yes | From `ci_analyze_pr_failures` |

### `ci_get_failed_logs`
Distilled failure logs for a run (error-extract, ~6KB cap per chunk).

| Param | Required | Notes |
|-------|----------|-------|
| `repo` | yes | |
| `run_id` | yes | |
| `offset_lines` | no | Line offset for paging (default **0**; use `next_offset_lines` from prior page) |
| `max_lines` | no | Lines per page (default **0** = single chunk; set e.g. **80** to page) |

### `ci_list_runs`
Recent Actions runs on a branch (main-guard, CI reports).

| Param | Required | Notes |
|-------|----------|-------|
| `repo` | yes | |
| `branch` | no | Branch name (default: repo default branch) |
| `limit` | no | Max runs (default **15**, max **50**) |

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

### `alert_list_open`
Open Dependabot security alerts (severity + summary).

| Param | Required | Notes |
|-------|----------|-------|
| `repo` | yes | |
| `limit` | no | Max alerts (default **20**) |

---

## Lazy meta-tools (MCP)

When the MCP server runs in `--lazy` mode, discover and call tools by name:

| Tool | Required `tool_args` | Optional |
|------|----------------------|----------|
| `tool_list` | (none) | |
| `tool_describe` | `name` | |
| `tool_call` | `name`, `args` (JSON object of the target tool's params) | |

---

## Harness (local Store — no GitHub API)

### `store_get_latest_digest`
Latest daily digest summary + pending approvals.

| Param | Required |
|-------|----------|
| (none) | |

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

### `issue_add_label`
Add a label to an issue (workflow / approval — not in default chat whitelist).

| Param | Required |
|-------|----------|
| `repo` | yes |
| `issue_number` | yes |
| `label` | yes |
