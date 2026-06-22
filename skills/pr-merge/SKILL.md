---
name: pr-merge
description: "Interpret merge blockers and review state. Use when the user asks if a PR can merge, who must approve, or why merge is blocked."
argument-hint: "PR number or merge-blocked list"
intent_keywords: [merge, block, approve, review]
intent_phrases: [needs to approve, merge blocked, waiting for review]
tools:
  - pr_get_merge_blockers
  - pr_get_review_state
  - pr_list_waiting_review
  - pr_list_merge_blocked
  - pr_list_merge_ready
  - pr_get_overview
---

# PR Merge

Report blockers verbatim from tools. Distinguish CI, approvals, conflicts, and policy labels.

## Scope

Use for:
- Merge readiness, review satisfaction, blocked vs waiting lists
- Draft PRs are out of scope for merge-ready scans

States to separate:
- **Waiting for review** — CI green, review not satisfied
- **Approved + red CI** — engineering work still needed
- **Blocked** — use `pr_get_merge_blockers` reasons as returned

## Workflow

1. **One PR** — `pr_get_merge_blockers` + `pr_get_review_state` (+ `pr_get_overview` for context).
2. **Repo scan** — `pr_list_merge_blocked`, `pr_list_waiting_review`, or `pr_list_merge_ready` as asked.
3. Summarize with PR link, structured blocker list, one-line headline.

## Output template

### Status
Merge-ready | blocked | waiting

### Blockers
Bullet list from tool output only

### Review state
Approvals, requested reviewers, CI summary
