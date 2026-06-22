---
name: ci-health
description: Branch and workflow CI health — not PR-specific. Use when user asks about main branch CI, workflow stats, recent runs, or regression after a deploy.
intent_keywords: [main, branch, workflow, ci, master, trunk, noisy, regression, deploy]
intent_phrases: [main branch ci, default branch, workflow stat, workflow stats]
intent_penalty_keywords: [pr, pull request]
intent_penalty: 6
tools:
  - ci_branch_health
  - ci_list_runs
  - ci_workflow_stats
  - ci_list_workflows
  - ci_correlate_prs
  - repo_get_info
---

## Tool chains

| Task | Chain |
|------|--------|
| Default branch health | `ci_branch_health` |
| Recent runs | `ci_list_runs` (optional `branch`) |
| Noisy workflows | `ci_workflow_stats` |
| Workflow names | `ci_list_workflows` |
| Who broke main | `ci_list_runs` (failing run) → `ci_correlate_prs` |

## Rules

- Branch omitted → repo default branch from `repo_get_info` or tool default.
- Prefer `ci_branch_health` rollup over manually counting `ci_list_runs` lines.
- PR-specific CI belongs in `ci-triage`, not here.
