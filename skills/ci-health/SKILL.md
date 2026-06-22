---
name: ci-health
description: "Branch and workflow CI health — not PR-specific. Use when the user asks about main/trunk CI, workflow noise, recent runs, or regressions after deploy."
argument-hint: "Branch name or workflow to inspect"
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

# CI Health

Roll up branch and workflow trends. PR-specific triage belongs in `ci-triage`.

## Scope

Use for:
- Default branch health, noisy workflows, “who broke main?”

Not for:
- Single PR check diagnosis → `ci-triage`

## Workflow

1. **Default branch** — `repo_get_info` if branch unknown; omit branch to use default.
2. **Rollup** — `ci_branch_health` over manually counting runs.
3. **Recent runs** — `ci_list_runs` (optional `branch`).
4. **Noisy workflows** — `ci_workflow_stats`.
5. **Workflow names** — `ci_list_workflows` when catalog is needed.
6. **Correlate** — failing run from `ci_list_runs` → `ci_correlate_prs`.

## Output template

### Branch health
Pass/fail trend, failing workflows (from tools)

### Recent failures
Run ID, workflow, time — one line each

### Suspected PRs (if correlated)
`#N` or “none from tools”
