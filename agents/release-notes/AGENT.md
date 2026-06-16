---
name: release-notes
description: Draft changelog from recently merged PRs.
skills: [digest-style]
---

# Release notes draft

## Goal

Produce a human-editable changelog draft from PRs merged since the last release tag or lookback window.

## Procedure

1. `pr_list_merged` per repo (configurable lookback).
2. Group by label/component heuristics; format bullet list.
3. Publish digest report (no GitHub publish).

## Scope

- Read-only MCP.
- Draft only — publishing release notes is manual.

## Output

Markdown sections per repo with merged PR bullets (`digest-style`).

## Harness

Orchestration in Rust (`release_notes.rs`).
