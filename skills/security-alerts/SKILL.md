---
name: security-alerts
description: Dependabot and code-scanning alerts. Use when user asks about security, dependabot, vulnerabilities, CVE, or alert severity.
intent_keywords: [security, dependabot, vulnerabilit, cve, alert, scanning]
tools:
  - alert_summarize_open
  - alert_list_open
  - repo_get_info
---

## Tool chains

| Task | Chain |
|------|--------|
| Dashboard rollup | `alert_summarize_open` |
| Full list | `alert_summarize_open` → `alert_list_open` if details needed |

## Rules

- Lead with severity counts (critical/high/medium/low) from summarize output.
- No dismiss, fix, or auto-PR actions — read-only.
- If zero alerts, say repo is clean per tool output (not “probably fine”).
- Link or name the package/CVE when the tool provides it.
