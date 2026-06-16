---
name: light-review
description: Map-Reduce light diff review for risk hints (no symbol RAG).
---

# Light review

## Goal

Flag risky patterns in a PR diff using capped Map-Reduce LLM passes — informational, not a merge gate.

## Procedure

1. `pr_list_changed_files` then `pr_get_diff` per file (size caps in harness).
2. Map: per-file risk hints; Reduce: top risks summary.
3. Publish digest **Light Review** — no GitHub review submission.

## Scope

- **No RAG / symbol index** — diff text only.
- Does not replace human code review.
- Read-only MCP except optional approved comments (disabled by default).

## Output

Digest with ranked risk bullets and file references.

## Harness

Orchestration in Rust (`light_review.rs`).
