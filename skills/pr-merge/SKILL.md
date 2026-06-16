---
name: pr-merge
description: Interpret merge blockers and review state.
---

## Merge blockers

- Use `pr_get_merge_blockers` output **verbatim** for blocker reasons — do not invent checks or reviewers.
- Distinguish: failing CI vs missing approvals vs branch conflicts vs policy labels.

## Review state

- **Waiting for review**: CI green but review not satisfied.
- **Approved** with red CI: engineering work still needed.
- Draft PRs are out of scope for merge-ready scans.

## Reporting

- Include PR link, structured blocker list, and a one-line summary.
