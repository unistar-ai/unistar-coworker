---
name: daily-work
description: Morning GitHub triage — list open PRs, analyze CI failures, classify flaky vs real, produce a daily digest.
---

# Daily Work with unistar-mcp

Use unistar-mcp tools (lazy mode: `tool_list` → `tool_describe` → `tool_call`).

## Tools

| Tool | Purpose |
|------|---------|
| `pr_list_open` | Open PRs with compact CI/review lines |
| `pr_get_status` | Single PR mergeability snapshot |
| `ci_analyze_pr_failures` | Failing runs + run IDs |
| `ci_get_failed_logs` | Distilled error lines; pass `max_lines` + `offset_lines` to page |
| `ci_rerun_workflow` | Rerun failed jobs (mutating — needs approval) |
| `pr_create_backport` | Backport merged PR (mutating — needs approval) |

## Per-repo workflow

1. `pr_list_open` for each configured repo (respect `limit`).
2. For PRs with failing CI: `pr_get_status` → `ci_analyze_pr_failures` → `ci_get_failed_logs` (paged).
3. Classify **flaky vs real bug** page-by-page; carry a short summary forward, not full log history.
4. Emit a ≤500 token summary per PR; reduce into the daily digest.

## Rules

- `action_required` is approval-waiting, not a code failure.
- External CI: if status fails but analyze finds no Actions runs, say so.
- One PR per agent session — do not batch logs for many PRs in one context.
- Mutating tools only after human approval in the TUI.
