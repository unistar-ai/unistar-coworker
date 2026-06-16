---
name: ci-efficiency
description: Aggregate default-branch CI run stats for efficiency review.
---

# CI efficiency report

## Goal

Summarize recent default-branch workflow runs: failure rate, duration trends, noisy workflows.

## Procedure

1. For each repo, `ci_list_runs` on default branch (`since_days` window).
2. Aggregate pass/fail counts and median duration per workflow (rules in Rust).
3. Publish digest report.

## Scope

- Read-only MCP; no reruns or workflow edits.
- No LLM — numeric aggregation only.

## Output

Digest **CI Efficiency** with per-workflow stats table.

## Harness

Orchestration in Rust (`ci_efficiency.rs`).
