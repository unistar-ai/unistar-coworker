---
name: oncall-handoff
description: Assemble on-call handoff pack from local Store.
skills: [digest-style]
---

# On-call handoff

## Goal

Produce a handoff markdown pack from Store snapshots: pending approvals, main alerts, recent digests, flaky highlights.

## Procedure

1. Read pending approvals, unacknowledged main alerts, latest digests, top flaky tests from Store.
2. Format scannable sections (see `digest-style` skill).
3. Publish digest / export for shift change.

## Scope

- Store-only reads — no live GitHub fetch required.
- Informational; no mutating actions.

## Output

Digest **On-call Handoff** with linked PRs, alert summaries, and open approval count.

## Harness

Orchestration in Rust (`oncall.rs`).
