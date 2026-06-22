---
name: oncall-store
description: Local Store — digests, pending approvals, on-call handoff. Use when user asks about pending approvals, latest digest, handoff, or what the coworker already recorded.
intent_keywords: [oncall, on-call, handoff]
intent_phrases: [pending approval, approval queue, latest digest, stored digest]
tools:
  - store_get_latest_digest
  - store_list_pending_approvals
  - store_get_oncall_handoff
---

## Tool chains

| Task | Chain |
|------|--------|
| Morning context | `store_get_latest_digest` |
| Approval queue | `store_list_pending_approvals` |
| Shift handoff | `store_get_oncall_handoff` |

## Rules

- Store is local — may lag live GitHub; say so when user expects real-time CI.
- Pending approvals need human action in TUI — list kind, repo, and description.
- If no digest yet, suggest running `daily-work` workflow.
