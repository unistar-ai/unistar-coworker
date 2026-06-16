---
name: pr-hygiene
description: Flag stale, docs-only, and oversized open PRs.
---

# PR hygiene scan

## Goal

Find open PRs that need cleanup: stale (no activity), docs-only with heavy CI, or unusually large diffs.

## Procedure

1. `pr_list_open` per repo.
2. Apply heuristics: days since update, changed-file count, label/title signals (Rust rules).
3. Append findings to digest.

## Scope

- Informational scan — no auto-close or comment.
- Thresholds from config `hygiene` section when set.

## Output

Digest **PR Hygiene** with finding lines per PR.

## Harness

Orchestration in Rust (`pr_hygiene.rs`).
