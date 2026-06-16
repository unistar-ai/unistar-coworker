---
name: release-duty
description: Scan merged PRs with backport label; queue approval-gated backports.
skills: [pr-merge]
---

# Release / backport duty

## Goal

Find recently merged PRs labeled for backport and queue target-branch backports for human approval.

## Procedure

1. For each repo, list merged PRs with `release.backport_label` (GitHub search, `release.lookback_limit`).
2. Confirm merged state via `pr_get_status`.
3. For each `release.target_branches` entry: skip if already queued; else push `BackportQueueItem` + `Approval` (unless `policy.auto_backport`).
4. Write markdown summary to audit log.

## Scope

- Requires `release.target_branches` in config.
- Backport execution is **approval-only** — never auto-merge.
- No LLM.

## Output

Markdown report: repo sections, queued/skipped lines, summary counts.

## Harness

Orchestration in Rust (`release.rs`).
