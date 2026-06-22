---
name: code-edit
description: Small, safe file edits — read before write, prefer precise patches over rewrites.
intent_keywords: [edit, fix, change, patch, refactor, rename, update, modify]
tools:
  - read_file
  - edit_file
  - write_file
  - grep
  - bash_run
---

## Principles

- **Read first** — always `read_file` the target region before `edit_file` or `write_file`.
- **Small steps** — one logical change per edit; avoid rewriting entire files when a patch suffices.
- **Exact match** — `edit_file` `old_string` must match the file (whitespace matters; LF/CRLF is normalized automatically when you copy from `read_file`).
- **Verify** — after mutating, run relevant tests or build commands via `bash_run`.

## `edit_file`

Required: `path`, `old_string`, `new_string`.

Optional:

- **`replace_all`** (boolean, default `false`) — replace every occurrence of `old_string`. Use when the same snippet appears many times and you intend to change all of them (e.g. rename a symbol). When `false`, the match must be **unique** or the tool returns `FILE_AMBIGUOUS_EDIT`; add surrounding lines to `old_string` or set `replace_all: true`.

`new_string` line endings are adjusted to match the file (CRLF repos stay CRLF). Binary / non-UTF-8 files are rejected — use `bash_run` instead.

## `write_file`

Required: `path`, `content`.

Optional:

- **`create_only`** (boolean, default `false`) — if `true`, fail with `FILE_EXISTS` when the path already exists. Use for new files to avoid accidental overwrites; use `edit_file` for changes to existing files.

On **overwrite**, `write_file` preserves the file’s line-ending style (LF vs CRLF) and trailing newline. On **create**, a trailing newline is added if missing. UTF-8 text only.

## Anti-patterns

- Blind `write_file` over an existing file without reading it.
- Large speculative rewrites when the user asked for a targeted fix.
- Multiple mutating tools in one turn (harness allows only one; plan accordingly).

## When to use `write_file`

- Creating a new file — prefer `create_only: true` so a typo in `path` does not clobber an existing file.
- Replacing a file only when a full rewrite is clearly simpler than several `edit_file` calls (omit `create_only` or set `create_only: false` to overwrite intentionally).
