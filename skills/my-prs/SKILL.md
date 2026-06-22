---
name: my-prs
description: "Author-focused open PR status — what needs fixing vs waiting on review. Use when the user asks about my PRs, my open pulls, or branches they own."
argument-hint: "Author filter or repo (defaults to @me when supported)"
intent_phrases: [my pr, my open, my pull, assigned to me, what do i need]
intent_bonus_keywords: ["@me"]
tools:
  - pr_list_open
  - pr_get_status_batch
  - pr_get_overview_batch
  - pr_get_overview
  - pr_get_ci_snapshot
---

# My PRs

Bucket PRs for the author: fix CI, wait for review, or ready to merge.

## Scope

Use for:
- Open PRs filtered by author (`author: "@me"` or login when tool supports it)
- Batch status across many PRs (respect tool limits: ≤15 status, ≤5 overview batch)

Do not invent author filters — pass `author` when the schema supports it.

## Workflow

1. **List** — `pr_list_open` with author filter; single-repo from config or scan configured repos.
2. **Batch status** — `pr_get_status_batch` on PR numbers (≤15).
3. **Deep dive on failures** — `pr_get_overview_batch` (≤5) or `pr_get_ci_snapshot` / `pr_get_overview` on one hot PR.
4. **Bucket** — CI failing → attention; review blocked → waiting; green + approved → ready.

## Output template

### Needs your action (CI / conflicts)
- `#N` — one line

### Waiting on others
- `#N` — review/approval state

### Ready
- `#N` — one line
