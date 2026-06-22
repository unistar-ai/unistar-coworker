---
name: external-ci
description: Non-GitHub Actions checks — Jenkins, Codecov, etc. Use when CI is red but Actions is green, or user mentions external check, Jenkins, codecov.
intent_keywords: [jenkins, codecov, external check]
intent_phrases: [external ci, actions green]
tools:
  - ci_list_external_checks
  - ci_get_check_url
  - ci_analyze_pr_failures
  - pr_get_ci_snapshot
  - pr_get_overview
---

## Tool chains

| Task | Chain |
|------|--------|
| PR external checks | `ci_analyze_pr_failures` (read CI_KIND) → `ci_list_external_checks` |
| Open details | `ci_get_check_url` |
| One-call snapshot | `pr_get_ci_snapshot` |

## Rules

- When `CI_KIND` is `external_only` or `pending`, **do not** call `ci_get_failed_logs`.
- Name the external check and URL from tool output.
- Mixed Actions + external → triage Actions with `ci-triage` tools separately.
