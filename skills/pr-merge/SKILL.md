---
name: pr-merge
description: Interpret merge blockers and review state. Use when user asks about merge, review, approval, or blocked PRs.
intent_keywords: [merge, block, approve, review]
intent_phrases: [needs to approve, merge blocked, waiting for review]
tools:
  - pr_get_merge_blockers
  - pr_get_review_state
  - pr_list_waiting_review
  - pr_list_merge_blocked
  - pr_list_merge_ready
  - pr_get_overview
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
