---
name: merge-health
description: Merge-ready PRs blocked by gates.
skills: [pr-merge]
---

# Merge health agent

## Goal

Find open PRs with **passing CI** that are still **not mergeable** — blocked by reviews, labels, conflicts, or policy.

## Procedure

1. `pr_list_open` per repo (respect `limit`).
2. Skip drafts and PRs with failing CI.
3. For CI-green PRs, call `pr_get_merge_blockers`.
4. If not mergeable, append digest line with structured blocker summary (`pr-merge` skill rules).

## Scope

- Scan only — no comments, reruns, or merges.
- Harness enforces read-only MCP; blockers text must come from MCP output.
- Orchestration in Rust (`merge_health.rs`).

## Output

Digest section with PR link, blocker detail, one-line summary. Empty state: _No theoretically-ready PRs blocked on merge gates._
