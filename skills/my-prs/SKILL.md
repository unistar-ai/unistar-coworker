---
name: my-prs
description: Author-focused open PR status. Use when user asks about my PRs, my open pulls, or what I need to fix on my branches.
intent_phrases: [my pr, my open, my pull, assigned to me, what do i need]
intent_bonus_keywords: ["@me"]
tools:
  - pr_list_open
  - pr_get_status_batch
  - pr_get_overview_batch
  - pr_get_overview
  - pr_get_ci_snapshot
---

## Tool chains

| Task | Chain |
|------|--------|
| My open list | `pr_list_open` with `author: "@me"` or user's login |
| Batch status | `pr_list_open` → `pr_get_status_batch` (≤15 numbers) |
| Batch overview | `pr_get_overview_batch` (≤5) for failing subset |
| One hot PR | `pr_get_ci_snapshot` or `pr_get_overview` |

## Rules

- Bucket for the user: **CI failing** → attention; **review blocked** → waiting; **green + approved** → ready.
- Use configured repo when single-repo; otherwise scan all configured repos.
- Do not invent author filter — pass `author` param when tool supports it.
