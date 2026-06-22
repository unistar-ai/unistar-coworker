---
name: repo-explore
description: Find code in the workspace with glob and grep before reading large files.
intent_keywords: [find, search, where, locate, explore, show, list, which]
tools:
  - glob
  - grep
  - read_file
---

## Search strategy

1. **`glob`** — narrow file candidates (`**/*.rs`, `src/**/*.ts`, etc.).
2. **`grep`** — search content within a directory or file pattern; prefer scoped paths over repo root.
3. **`read_file`** — read only the lines you need (`start_line`, `max_lines`).

## Rules

- Do not read entire large files when grep/glob can pinpoint the location.
- Prefer relative paths under `chat.workspace`; all paths are sandboxed.
- When explaining structure, cite paths and line ranges from tool output.

## Anti-patterns

- Reading every file in a directory sequentially.
- Grepping the whole repo without a pattern or path hint when the user named a module.
