---
name: daily-work
description: Morning GitHub triage digest across configured repos.
skills: [ci-triage, digest-style]
---

# Daily work agent

## Goal

Produce daily digest: open PRs, classify failing CI, split attention / flaky / policy.

## Procedure

1. `pr_list_open` per configured repo (respect `limit`).
2. For PRs with failing CI: harness runs `triage_pr` (overview → analyze → summary → paged logs → LLM classify) per PR.
3. For CI-green PRs waiting on review: record in digest.
4. Publish digest to Store.

## Scope

- One PR per triage session for log context — do not batch logs for many PRs in one LLM call.
- Mutating tools → approval only (harness enforces).

## Tools

Use unistar-mcp lazy mode when ad-hoc: `tool_list` → `tool_describe` → `tool_call`.  
See `skills/_base/TOOLS.md` for the base read-only set.
