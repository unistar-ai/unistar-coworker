---
name: issue-tracker
description: "List, search, and inspect GitHub issues. Use when the user asks about open bugs, tickets, labels, or issue search."
argument-hint: "Repo, labels, or search query"
intent_keywords: [issue, bug, ticket]
intent_phrases: [open bugs, open issues]
tools:
  - issue_list_open
  - issue_get
  - issue_search
  - repo_get_info
---

# Issue Tracker

Summarize issues concisely. Full bodies only when the user asks.

## Scope

Use for:
- Open backlog, single issue detail, keyword search

Mutating actions (`issue_add_label`, comments) require approval — not in this skill’s default flow.

## Workflow

1. **Open backlog** — `issue_list_open`.
2. **One issue** — `issue_get` with number.
3. **Search** — `issue_search` with GitHub query syntax (`is:open label:bug`).
4. **Labels** — `repo_get_info` for label names when queries need them.

## Output template

### Issues
`#N` title — labels — author (one line each)

### Detail (if single issue)
State, labels, assignees, short body summary
