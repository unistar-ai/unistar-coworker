---
name: release-duty
description: Scan merged PRs needing backport; queue backport PRs for TUI approval.
---

# Release / Backport duty

1. Find merged PRs labeled `needs-backport` (or configured label).
2. Match target release branches from config.
3. Queue `pr_create_backport` actions for TUI approval — never auto-push without policy.
