---
name: security-digest
description: Dependabot and code-scanning alert summary per repo.
skills: [digest-style]
---

# Security digest

## Goal

Summarize open Dependabot / code-scanning alerts so security debt is visible on the Dashboard.

## Procedure

1. `alert_list_open` per configured repo.
2. Group by severity; cap lines per repo for digest size.
3. Publish digest **Security Digest**.

## Scope

- Read-only — no dismiss or fix PR creation.
- Alerts without repo scope are skipped.

## Output

Digest section with severity counts and top alert lines with links.

## Harness

Orchestration in Rust (`security_digest.rs`).
