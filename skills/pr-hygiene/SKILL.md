---
name: pr-hygiene
description: Stale, oversized, and docs-only PR hygiene. Use when user asks about stale PRs, mega PRs, cleanup, or PR housekeeping.
intent_keywords: [stale, mega, hygiene, housekeeping, cleanup]
intent_phrases: [large pr, docs-only, docs only, oversized]
tools:
  - pr_list_stale
  - pr_list_large
  - pr_is_docs_only
  - pr_diff_risk_scan
  - pr_list_open
---

## Tool chains

| Task | Chain |
|------|--------|
| Idle PRs | `pr_list_stale` (optional `days`) |
| Mega PRs | `pr_list_large` → `pr_diff_risk_scan` on hits |
| Docs-only check | `pr_is_docs_only` |
| Full open scan | `pr_list_open` then filter heuristically |

## Rules

- Informational — no auto-close or nudge comments without approval.
- Stale threshold default 7 days; mention the threshold used.
- Large PR thresholds: default 30 files / 1000 lines — cite tool output.
