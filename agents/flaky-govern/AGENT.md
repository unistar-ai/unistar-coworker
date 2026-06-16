---
name: flaky-govern
description: Top flaky tests from local ledger; quarantine hints.
---

# Flaky test governance

## Goal

Summarize flaky test incidents from the local Store ledger for weekly review and quarantine decisions.

## Procedure

1. Query `store.list_flaky_tests` (30-day window, top 20).
2. Build markdown table: test name, repo, workflow, incident count, rerun pass rate.
3. Publish digest report to Store + optional export.

## Scope

- **Harness-only** — no MCP; data comes from prior triage `record_flaky_incident` entries.
- User reclassification via TUI `[6] Flaky` is out of band.

## Output

Digest **Flaky Govern** with table or _No flaky incidents recorded yet._

## Harness

Orchestration in Rust (`flaky_govern.rs`).
