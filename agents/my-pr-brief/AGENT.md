---
name: my-pr-brief
description: Author-focused digest of your open PRs across repos.
skills: [ci-triage, digest-style]
---

# My PR brief

## Goal

Give the configured GitHub user a compact view of **their** open PRs: CI status, review state, merge readiness.

## Procedure

1. `pr_list_open` per repo; filter lines where author matches configured user.
2. Bucket: CI failing → attention; review blocked → waiting; CI green + approved → ready.
3. Publish digest **My PR Brief** with counts per bucket.

## Scope

- Author filter from config / GitHub identity — not all open PRs.
- Failing CI lines reference triage verdicts when available in Store; no full re-triage unless invoked separately.

## Output

Scannable digest with PR links grouped by bucket (`digest-style`).

## Harness

Orchestration in Rust (`my_pr_brief.rs`).
