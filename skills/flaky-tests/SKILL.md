---
name: flaky-tests
description: Flaky test investigation via CI tools and policy classification. Use when user asks about flaky tests, quarantine, rerun pass rate, or intermittent failures.
intent_keywords: [flaky, quarantine, intermittent, fingerprint]
intent_phrases: [rerun pass, intermittent failure]
tools:
  - ci_failure_fingerprint
  - ci_compare_runs
  - policy_classify_failure
  - ci_get_failure_digest
  - ci_get_run_summary
---

## Tool chains

| Task | Chain |
|------|--------|
| Fingerprint | `ci_get_run_summary` → `ci_failure_fingerprint` |
| Rerun compare | `ci_compare_runs` with run_id_a (fail) and run_id_b (pass) |
| Classify | `ci_failure_fingerprint` → `policy_classify_failure` |

## Rules

- Use live CI evidence from GitHub tools — digest `flaky_candidates` counts are summaries, not a persistent ledger.
- Timeout/infra class from `policy_classify_failure` often indicates flake vs real bug.
- Suggest `ci_rerun_workflow` only with user approval after showing evidence.
