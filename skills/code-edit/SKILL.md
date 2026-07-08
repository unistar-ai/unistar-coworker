---
name: code-edit
description: "Workspace coding workflow: explore, patch, verify. Use for fixes, features, refactors, and new files in chat.workspace."
argument-hint: "What to change and where"
intent_keywords: [edit, fix, change, patch, refactor, rename, update, modify, implement, add, test]
tools:
  - read_file
  - edit_file
  - write_file
  - grep
  - glob
  - bash_run
---

# Code Edit

End-to-end workspace coding: **explore → read → patch → verify → report**. Correctness beats speed; one logical mutating step per harness turn when editing.

## Scope

Use for:
- Targeted fixes, features, and small refactors in tracked files
- Creating new files with explicit intent
- Running tests, builds, or linters after changes

Do not:
- Blind `write_file` over existing files without reading
- Large speculative rewrites when a patch suffices
- Multiple mutating tools in one turn (harness allows one; plan ahead)
- Claim success without running a relevant check when one exists

## Workflow

1. **Orient** — clarify goal; `glob` / `grep` if paths are unknown.
2. **Read** — `read_file` the exact region; match whitespace when copying into `old_string`.
3. **Patch** — `edit_file` with unique `old_string` → `new_string`; use `replace_all` only when intentional.
4. **Create** — `write_file` with `create_only: true` for new files.
5. **Verify** — `bash_run` the project's test, build, or lint command (discover from README, Makefile, or `package.json`).
6. **Report** — summarize what changed and verification outcome; if verify failed, iterate from step 2.

## Tool notes

- **`edit_file`**: `old_string` must be unique unless `replace_all: true`. LF/CRLF normalized when copied from `read_file`. Binary rejected.
- **`write_file`**: preserves line endings on overwrite; adds trailing newline on create. UTF-8 text only.
- **Parallel reads**: `grep` + multiple `read_file` in one turn is fine before the first edit.

## Output template

### Change
- Path(s) and what changed (one line each)

### Verification
- Command run and result (exit code / pass-fail)

If blocked (ambiguous edit, binary file, missing test command): state the blocker and the smallest next read or command.
