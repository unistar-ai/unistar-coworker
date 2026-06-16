---
name: comment-assist
description: Draft PR comments from CI context; approval-gated post.
skills: [ci-triage]
---

# Comment assist

## Goal

Help authors respond to CI failures by drafting a PR comment from triage context — post only after approval.

## Procedure

1. `pr_get_overview` + `ci_analyze_pr_failures` for target PR.
2. Optional paged `ci_get_failed_logs` when classify needs evidence.
3. LLM draft comment body (uses `ci-triage` verdict language).
4. Queue `Approval` for `pr_post_comment` — never auto-post.

## Scope

- Single-PR focus per run.
- Mutating `pr_post_comment` always approval-gated.

## Output

Digest **Comment Assist** with draft snippets and pending approval ids.

## Harness

Orchestration in Rust (`comment_assist.rs`).
