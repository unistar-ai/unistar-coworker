---
name: breaking-sniff
description: Path/diff heuristics for breaking API changes; optional LLM on capped diff.
---

# Breaking change sniff

## Goal

Flag PRs that may introduce breaking API or config changes before merge.

## Procedure

1. `pr_list_open` or targeted PR from config.
2. Path heuristics (`BREAKING`, semver paths, public API dirs) on changed files.
3. Optional LLM pass on capped diff when heuristic hits.

## Scope

- Advisory only — not a merge blocker.
- Diff size caps enforced in harness.

## Output

Digest **Light Review** layout with risk lines per PR.

## Harness

Orchestration in Rust (`breaking_sniff.rs`).
