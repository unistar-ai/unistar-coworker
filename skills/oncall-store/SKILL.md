---
name: oncall-store
description: "Read local coworker store — latest digest, pending approvals, on-call handoff. Use when the user asks what was recorded, what's queued for approval, or shift handoff context."
argument-hint: "Digest, approvals, or handoff"
intent_keywords: [oncall, on-call, handoff]
intent_phrases: [pending approval, approval queue, latest digest, stored digest]
tools:
  - store_get_latest_digest
  - store_list_pending_approvals
  - store_get_oncall_handoff
---

# Oncall Store

Local store may lag live GitHub. Say so when the user expects real-time CI.

## Scope

Use for:
- Latest stored digest
- Pending approval queue (human action in TUI/Web UI)
- On-call handoff notes

## Workflow

1. **Morning context** — `store_get_latest_digest`.
2. **Approval queue** — `store_list_pending_approvals` (kind, repo, description).
3. **Shift handoff** — `store_get_oncall_handoff`.
4. If no digest exists, suggest chat (e.g. “triage open PRs in owner/repo”) or `triage-pr`.

## Output template

### Store snapshot
What was found (or empty)

### Pending actions
Bullets for approvals the human must resolve

### Staleness note
If live GitHub may differ from store
