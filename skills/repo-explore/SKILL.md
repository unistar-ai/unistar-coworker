---
name: repo-explore
description: "Find code and structure in the workspace before reading large files. Use when the user asks where something lives, which files match a pattern, or how the repo is organized."
argument-hint: "Symbol, path hint, or feature name to locate"
intent_keywords: [find, search, where, locate, explore, show, list, which]
tools:
  - glob
  - grep
  - read_file
---

# Repo Explore

Search narrowly, then read surgically. Avoid loading whole trees into context.

## Scope

Use for:
- Finding files, symbols, and references under `chat.workspace`
- Answering “where is X?” with paths and line ranges

Do not:
- Read entire large files when `grep` can pinpoint a region
- Grepping the whole repo without a pattern when the user named a module

## Workflow

1. **`glob`** — narrow candidates (`**/*.rs`, `src/**/*.ts`, etc.).
2. **`grep`** — content search with scoped path or pattern.
3. **`read_file`** — only needed lines (`start_line`, `max_lines`).
4. **Explain** — cite paths and line ranges from tool output.

## Output template

### Locations
- `path:line` — one-line why it matters

### Structure (if asked)
Short map of relevant dirs/files only — no exhaustive listing.
