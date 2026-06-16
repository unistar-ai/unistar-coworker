---
name: ci-triage
description: Classify CI failures — flaky, real bug, or policy gate.
---

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
- Read `ci_get_run_summary` before full logs; page through logs before concluding `unknown`.
- When explaining CI to users: distinguish flaky vs real vs policy in plain language.
