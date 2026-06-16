---
name: issue-triage
description: Index open issues across configured repos.
skills: [digest-style]
---

# Issue triage

## Goal

List and persist open issues per repo for TUI `[8] Issues` and digest visibility.

## Procedure

1. `issue_list_open` per repo (respect `limit`).
2. Parse compact lines; upsert `IssueSnapshot` with labels and updated date.
3. Append digest lines: `#N title (@author) — labels`.

## Tools

| Tool | When |
|------|------|
| `issue_list_open` | Enumerate open issues |
| `issue_get` | Optional detail fetch (Tier B; not in default scan loop) |

## Scope

- Read-only in scan loop.
- `issue_post_comment` / label changes → approval only (not invoked here).

## Output

Digest per repo; summary count of indexed issues.

## Harness

Orchestration in Rust (`issue_triage.rs`).
