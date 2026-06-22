---
name: pr-review
description: Light PR review — changed files, diff risk, CODEOWNERS routing. Use when user asks to review a PR, scan diff, code changes, risk, or who should review.
intent_keywords: [diff, risk, codeowner, routing, review, pr, pull, analyze, 分析, 审查]
intent_phrases: [code change, changed file, scan diff, review this pr, analyze this pr, analyze pr, 分析 pr, 分析这个, review this pull]
intent_bonus_keywords: [github.com, /pull/, "#"]
intent_penalty_phrases: []
tools:
  - pr_list_changed_files
  - pr_get_diff
  - pr_diff_risk_scan
  - pr_get_review_routing
  - pr_get_review_state
  - pr_get_overview
---

## Tool chains

| Task | Chain |
|------|--------|
| Quick scope | `pr_get_overview` → `pr_list_changed_files` |
| Risk flags | `pr_list_changed_files` → `pr_diff_risk_scan` |
| Patch detail | `pr_list_changed_files` → `pr_get_diff` (use `max_bytes` if huge) |
| Who to ping | `pr_get_review_routing` → `pr_get_review_state` |

## Rules

- Informational only — do not submit GitHub reviews unless user explicitly requests an approved mutating action.
- Prefer `pr_diff_risk_scan` before reading full `pr_get_diff` on large PRs.
- Report lockfile, workflow, migration, and line-count flags from tools verbatim.
- If diff is truncated, say so; do not infer unseen hunks.
