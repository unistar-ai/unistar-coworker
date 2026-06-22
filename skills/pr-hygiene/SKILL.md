---
name: pr-hygiene
description: "Stale, oversized, and docs-only PR housekeeping. Use when the user asks about stale PRs, mega PRs, cleanup, or docs-only detection."
argument-hint: "Stale days threshold or size criteria"
intent_keywords: [stale, mega, hygiene, housekeeping, cleanup]
intent_phrases: [large pr, docs-only, docs only, oversized]
tools:
  - pr_list_stale
  - pr_list_large
  - pr_is_docs_only
  - pr_diff_risk_scan
  - pr_list_open
---

# PR Hygiene

Informational scans only — no auto-close or nudge comments without approved mutating actions.

## Scope

Use for:
- Idle PRs, large PRs, docs-only classification, housekeeping reports

## Workflow

1. **Stale** — `pr_list_stale` (default 7 days — state threshold used).
2. **Large** — `pr_list_large` (default 30 files / 1000 lines — cite tool thresholds).
3. **Risk on hits** — `pr_diff_risk_scan` on mega PR candidates when useful.
4. **Docs-only** — `pr_is_docs_only` per PR when asked.
5. **Full open scan** — `pr_list_open` then filter heuristically if needed.

## Output template

### Stale (>{N}d)
- `#N` title — last activity hint

### Large / risky
- `#N` — files/lines + risk flags from tools

### Docs-only (if checked)
- `#N` yes/no per tool
