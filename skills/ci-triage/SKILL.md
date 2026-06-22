---
name: ci-triage
description: Classify CI failures — flaky, real bug, or policy gate. Use when user asks about CI, builds, failing checks, tests, or PR status.
intent_keywords: [ci, fail, flaky, build, workflow, test]
intent_bonus_keywords: [pr, pull, "#"]
tools:
  - pr_get_ci_snapshot
  - pr_get_overview
  - ci_get_failure_digest
  - ci_get_failed_logs
  - ci_analyze_pr_failures
  - ci_get_run_summary
  - ci_failure_fingerprint
  - policy_classify_failure
  - harness_triage_pr
---

## Tool chains (names only — params via tool_call or warmed schema)

| Task | Chain |
|------|--------|
| PR CI overview | `pr_get_ci_snapshot` or `resource_read` `github://pull/{owner}/{repo}/{n}/ci-snapshot` |
| Failure analysis | `ci_get_failure_digest` → (if needed) `ci_get_failed_logs` |
| PR + failing runs | `ci_analyze_pr_failures` → `ci_get_run_summary` |
| Flaky check | `ci_failure_fingerprint` → `policy_classify_failure` |

## Verdicts

- **flaky**: transient infra/network/timeouts; rerun may pass
- **real**: code/test/build bug in the PR
- **policy**: labels, approvals, changelog, or template gates — **not** engineering attention; author needs a person/label/approval, not a code fix
- **unknown**: logs insufficient on this page

Policy examples: manager approval checker, changelog requirement, PR template regex, missing labels.
Do **not** use verdict `real` for those — they belong in `policy`.

## Rules

- `action_required` is approval-waiting, not a code failure.
- External CI: if status fails but GitHub Actions analyze finds no runs, say so explicitly.
- Prefer `pr_get_ci_snapshot` or `ci_get_failure_digest` before paged `ci_get_failed_logs`.
- Read `ci_get_run_summary` when you need run metadata before deeper logs.
- When explaining CI to users: distinguish flaky vs real vs policy in plain language.
