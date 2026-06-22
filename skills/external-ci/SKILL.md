---
name: external-ci
description: "Non-GitHub Actions checks — Jenkins, Codecov, third-party statuses. Use when CI is red but Actions is green, or the user names an external check."
argument-hint: "PR or check name (Jenkins, Codecov, etc.)"
intent_keywords: [jenkins, codecov, external check]
intent_phrases: [external ci, actions green]
tools:
  - pr_get_ci_snapshot
  - ci_analyze_pr_failures
  - ci_list_external_checks
  - ci_get_check_url
  - pr_get_overview
---

# External CI

When `CI_KIND` is external-only or pending, do not call `ci_get_failed_logs` for Actions-style log mining.

## Scope

Use for:
- PR checks outside GitHub Actions
- Mixed Actions + external — triage Actions with `ci-triage` separately

## Workflow

1. **Snapshot** — `pr_get_ci_snapshot` or `pr_get_overview`.
2. **Analyze** — `ci_analyze_pr_failures`; read `CI_KIND` from output.
3. **External list** — `ci_list_external_checks`.
4. **URLs** — `ci_get_check_url` for user-facing links.
5. Name each failing external check and URL from tool output verbatim.

## Output template

### CI kind
From tool output (`actions_only`, `external_only`, `mixed`, etc.)

### External checks
Name — status — URL

### Next step
Open URL, fix external job, or escalate — one action
