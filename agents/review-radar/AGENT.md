---
name: review-radar
description: List PRs that are CI-green but blocked on review.
skills: [pr-merge, digest-style]
---

# Review blocker radar

## Goal

Surface open PRs that pass CI but still need human review — so reviewers know what to pick up.

## Procedure

1. For each configured repo, call `pr_list_waiting_review` (respect `policy.max_prs_per_repo`).
2. Each line is pre-filtered: CI passing, review required, not draft.
3. Upsert PR snapshots with `triage_note: review blocked`.
4. Publish digest section **Waiting for review** with PR links and authors.

## Scope

- Do **not** re-filter with `pr_list_open` when `pr_list_waiting_review` is available.
- No LLM — harness orchestrates MCP + Store only.
- Mutating tools → approval only (not used in this workflow).

## Output

Digest titled **Review Radar**; summary counts waiting PRs per repo. One line per PR: `#N title — link (@author)`.

## Harness

Step orchestration in Rust (`review_radar.rs`).
