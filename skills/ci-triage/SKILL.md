---
name: ci-triage
description: "Classify CI failures on a PR or run — flaky, real bug, policy gate, or unknown. Use when checks are red, builds fail, tests fail, or the user asks why CI is failing."
argument-hint: "PR number, run URL, or failing check name"
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

# CI Triage

Diagnose CI with evidence from harness tools. Separate engineering failures from policy gates and infra flakes.

## Scope

Use for:
- PR check failures and run-level failures
- “Why is CI red?” on a specific PR or workflow run

Not for:
- Default-branch health rollups → `ci-health`
- External-only CI patterns → `external-ci` first

## Workflow

1. **Anchor** — `repo`, `pr_number`, and/or `run_id` from user text or URL.
2. **PR snapshot** — `pr_get_ci_snapshot` or `pr_get_overview` for check rollup.
3. **Failure digest** — `ci_get_failure_digest` before paging logs.
4. **Run context** — `ci_get_run_summary` when metadata is needed before deep logs.
5. **PR + runs** — `ci_analyze_pr_failures` when multiple runs/checks need correlation.
6. **Flake vs real** — `ci_failure_fingerprint` → `policy_classify_failure` when classification is unclear.
7. **Logs last** — `ci_get_failed_logs` only when digest/summary is insufficient (paged; cap reads).

## Verdicts

| Verdict | Meaning |
|---------|---------|
| **flaky** | Transient infra/network/timeouts; rerun may pass |
| **real** | Code, test, or build bug in the change |
| **policy** | Labels, approvals, changelog, templates — needs human/process action, not a code fix |
| **unknown** | Logs insufficient on this page |

`action_required` is approval-waiting, not a code failure. Do **not** use `real` for manager-approval, changelog, or label gates.

## Output template

### Verdict
**flaky** | **real** | **policy** | **unknown**

### Evidence
- Check/run names, first error line, tool fields cited

### Next step
One concrete action (fix, rerun, label, ping reviewer, or fetch more logs)

If external CI fails but Actions analysis finds no runs, say so explicitly.
