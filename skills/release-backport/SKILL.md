---
name: release-backport
description: "Releases, tags, merged PRs, and backport candidates. Use when the user asks about release notes, tags, changelog, backport, or cherry-pick planning."
argument-hint: "Tag, release window, or backport label"
intent_keywords: [release, tag, changelog, backport, cherry-pick, cherry pick]
intent_phrases: [merged pr]
tools:
  - release_list_tags
  - release_notes_draft
  - pr_list_merged
  - pr_list_backport_candidates
  - pr_get_status
  - repo_get_info
---

# Release & Backport

Draft and plan from tool output. Mutating backport creation requires explicit user approval.

## Scope

Use for:
- Tag history, release note drafts, merged PR windows, backport queues

`pr_create_backport` is mutating — suggest only; do not call without user intent.

## Workflow

1. **Tags** — `release_list_tags`.
2. **Notes draft** — `release_notes_draft` since a chosen tag.
3. **Backport queue** — `pr_list_backport_candidates` → `pr_get_status` per candidate.
4. **Merged window** — `pr_list_merged` with `since` / `label`.
5. Confirm backport label (default `needs-backport`) from repo labels when unclear.

## Output template

### Tags / release window
From tools

### Backport candidates
`#N` — status — one line each

### Draft notes (if generated)
Paste or summarize — user edits before publish
