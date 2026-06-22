---
name: pr-review
description: "Review pull requests for bugs, regressions, risk flags, and review routing. Use when the user asks for a PR review, diff inspection, changed-file scan, risk assessment, or who should review. Prefer this over implementation skills when the task is review."
argument-hint: "PR number, URL, or repo + what to review"
intent_keywords: [diff, risk, codeowner, routing, review, pr, pull, analyze, 分析, 审查]
intent_phrases: [code change, changed file, scan diff, review this pr, analyze this pr, analyze pr, 分析 pr, 分析这个, review this pull]
intent_bonus_keywords: [github.com, /pull/, "#"]
tools:
  - pr_get_overview
  - pr_list_changed_files
  - pr_diff_risk_scan
  - pr_get_diff
  - pr_get_review_routing
  - pr_get_review_state
---

# PR Review

Review with a bug-finding mindset. Prioritize correctness, security, breaking changes, and tool-reported risk over style.

Harness tools supply scope, risk heuristics, and routing — not a substitute for reading the diff when the user needs code-level findings.

## Scope

Use for:
- GitHub pull requests (overview, files, diff, risk scan)
- Merge-readiness signals and review routing
- Pre-merge checks grounded in tool output

Do not:
- Post GitHub reviews or comments unless the user explicitly requests an approved mutating action
- Invent hunks, CI state, or reviewers when tools truncate or omit data
- Turn the task into a general refactor or nit-pick pass

## Workflow

1. **Anchor the PR** — `repo` + `pr_number` from the URL or user text.
2. **Scope first** — `pr_get_overview` → `pr_list_changed_files`.
3. **Risk before full diff** — on large PRs, `pr_diff_risk_scan` before `pr_get_diff` (set `max_bytes` when needed).
4. **Patch detail** — `pr_get_diff` only when risk scan or the user needs line-level evidence.
5. **People** — `pr_get_review_routing` → `pr_get_review_state` when the question is who should review or approval status.
6. **Review angles** — incorrect vs intent, security/permissions, breaking API/config/migrations, reliability, hot-path performance, repo guidance in changed paths.
7. **Strict findings** — actionable, evidenced, file/line when available; skip style nits unless asked.

## Findings format

Order by severity. For each finding: short title, why it matters, concrete evidence, file/line or URL.

If tools truncate the diff, say so — do not infer unseen hunks. Report lockfile, workflow, migration, and line-count flags from tools verbatim.

## Output template

### Findings
1. ...

### Open questions
- ...

### Summary
- ...

If no material issues: **No material issues found.** plus **Residual risk:** ...
