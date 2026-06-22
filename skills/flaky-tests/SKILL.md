---
name: flaky-tests
description: "Investigate intermittent CI failures — fingerprints, run comparison, flake classification. Use when the user mentions flaky tests, quarantine, rerun pass rate, or intermittent failures."
argument-hint: "Run IDs, workflow name, or PR with flaky check"
intent_keywords: [flaky, quarantine, intermittent, fingerprint]
intent_phrases: [rerun pass, intermittent failure]
tools:
  - ci_get_run_summary
  - ci_failure_fingerprint
  - ci_compare_runs
  - policy_classify_failure
  - ci_get_failure_digest
---

# Flaky Tests

Treat flake as a hypothesis backed by CI evidence — not by gut feel or stale digest counts alone.

## Scope

Use for:
- Intermittent failures, rerun comparisons, timeout/infra class signals

Do not:
- Treat digest `flaky_candidates` counts as a live ledger without run evidence
- Suggest `ci_rerun_workflow` without user approval after showing evidence

## Workflow

1. **Run context** — `ci_get_run_summary` for the failing (and passing) run.
2. **Fingerprint** — `ci_failure_fingerprint` on the failure signature.
3. **Compare** — `ci_compare_runs` with fail run vs pass rerun when both exist.
4. **Classify** — `policy_classify_failure`; timeout/infra class often indicates flake.
5. **Digest** — `ci_get_failure_digest` when you need condensed log lines first.

## Output template

### Assessment
**likely flaky** | **likely real** | **inconclusive**

### Evidence
Fingerprints, run IDs, classifier output

### Recommendation
Rerun, quarantine, fix code, or gather another run — one step only
