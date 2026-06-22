---
name: security-alerts
description: "Dependabot and code-scanning alerts — open summary and detail. Use when the user asks about vulnerabilities, CVEs, Dependabot, or alert severity."
argument-hint: "Repo or severity filter"
intent_keywords: [security, dependabot, vulnerabilit, cve, alert, scanning]
tools:
  - alert_summarize_open
  - alert_list_open
  - repo_get_info
---

# Security Alerts

Read-only. Lead with severity counts; no dismiss or auto-fix actions.

## Scope

Use for:
- Open alert dashboards and detailed lists

## Workflow

1. **Rollup** — `alert_summarize_open` (critical/high/medium/low).
2. **Detail** — `alert_list_open` when the user needs full list.
3. Cite package/CVE names when tools provide them.
4. If zero alerts, report clean **per tool output** — not “probably fine”.

## Output template

### Summary
Severity counts from tools

### Top alerts (if listed)
Package/CVE — severity — one line each

### Next step
Patch, ignore policy, or escalate — only if user asks
