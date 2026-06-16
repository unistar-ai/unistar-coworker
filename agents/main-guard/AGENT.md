---
name: main-guard
description: Watch default-branch CI; alert immediately when main goes red.
skills: [ci-triage]
---

# Main branch guard

## Goal

Detect consecutive default-branch CI failures early and write `main_alert` records for the Dashboard.

## Procedure

1. For each repo, call `ci_list_runs` on the default branch (`main_guard.recent_runs` limit).
2. Parse runs; compute leading failure streak (rules-first — no LLM).
3. When streak ≥ `main_guard.consecutive_failures`, upsert `MainAlert` and append to digest.
4. Publish incremental digest for TUI visibility.

## Scope

- Pure red/green detection — do not classify flaky vs real here (see `ci-triage` for PR-scoped triage).
- Runs on cron (`schedule.main_guard` or workflow schedule).
- No mutating MCP tools.

## Output

Digest **Main Guard** with alert lines: repo, branch, streak count, latest run link + workflow name.

## Harness

Orchestration in Rust (`main_guard.rs`).
