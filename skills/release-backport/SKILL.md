---
name: release-backport
description: Releases, tags, merged PRs, and backport candidates. Use when user asks about release, tag, changelog, backport, or cherry-pick.
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

## Tool chains

| Task | Chain |
|------|--------|
| Recent tags | `release_list_tags` |
| Notes since tag | `release_list_tags` → `release_notes_draft` |
| Backport queue | `pr_list_backport_candidates` → `pr_get_status` per candidate |
| Merged window | `pr_list_merged` with `since` / `label` |

## Rules

- `pr_create_backport` is mutating — approval only; suggest it, do not call without user intent.
- Backport label defaults to `needs-backport`; confirm from config/repo labels when unclear.
- Release notes draft is a starting point — user edits before publish.
