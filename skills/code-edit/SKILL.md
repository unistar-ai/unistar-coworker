---
name: code-edit
description: "Make small, safe file edits in the workspace. Use when the user wants a targeted fix, patch, rename, or new file — read before write, prefer edit_file over full rewrites."
argument-hint: "File path and what to change"
intent_keywords: [edit, fix, change, patch, refactor, rename, update, modify]
tools:
  - read_file
  - edit_file
  - write_file
  - grep
  - bash_run
---

# Code Edit

Change code in minimal, verifiable steps. Correctness beats speed; one logical change per edit round when mutating.

## Scope

Use for:
- Targeted fixes and small refactors in tracked files
- Creating new files with explicit intent

Do not:
- Blind `write_file` over existing files without reading
- Large speculative rewrites when a patch suffices
- Multiple mutating tools in one turn (harness allows one; plan ahead)

## Workflow

1. **Locate** — `grep` / `glob` if path is unclear.
2. **Read** — `read_file` the exact region; match whitespace when copying into `old_string`.
3. **Patch** — `edit_file` with unique `old_string` → `new_string`; use `replace_all` only when intentional.
4. **Create** — `write_file` with `create_only: true` for new files.
5. **Verify** — `bash_run` the relevant test, build, or lint command.

## Tool notes

- **`edit_file`**: `old_string` must be unique unless `replace_all: true`. LF/CRLF normalized when copied from `read_file`. Binary rejected.
- **`write_file`**: preserves line endings on overwrite; adds trailing newline on create. UTF-8 text only.

## Output template

### Change
- Path(s) and what changed (one line each)

### Verification
- Command run and result (exit code / pass-fail)

If blocked (ambiguous edit, binary file): state the blocker and the smallest next read or command.
