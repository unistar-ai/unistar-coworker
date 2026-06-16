---
name: regression-link
description: Correlate failing tests with recently merged PRs (informational).
skills: [digest-style]
---

# Regression link

## Goal

Surface candidate PRs that may have caused a failing test on default branch — correlation only, not blame.

## Procedure

1. Read latest main-branch failure from Store / `ci_list_runs`.
2. `pr_list_merged` in lookback window; match touched paths / test names heuristically.
3. Publish informational digest section.

## Scope

- Heuristic links — verify manually before revert.
- No auto-revert or bisect.

## Output

Digest **Release Notes** layout with candidate PR list and confidence hints.

## Harness

Orchestration in Rust (`regression_link.rs`).
