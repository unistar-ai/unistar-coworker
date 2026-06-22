---
name: issue-tracker
description: Open issues — list, search, and detail. Use when user asks about issues, bugs, tickets, labels, or GitHub issue search.
intent_keywords: [issue, bug, ticket]
intent_phrases: [open bugs, open issues]
tools:
  - issue_list_open
  - issue_get
  - issue_search
  - repo_get_info
  - harness_run_workflow
---

## Tool chains

| Task | Chain |
|------|--------|
| Open backlog | `issue_list_open` |
| One issue | `issue_get` |
| Keyword search | `issue_search` with GitHub query syntax |
| Label context | `repo_get_info` (label names) → `issue_search` |

## Rules

- `issue_search` `query` uses GitHub search syntax (e.g. `is:open label:bug`).
- Summarize title, number, labels, and author — do not paste full bodies unless asked.
- Mutating actions (`issue_add_label`, comments) require approval — not in this skill chain.
