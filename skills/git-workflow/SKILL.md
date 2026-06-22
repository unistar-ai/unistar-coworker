---
name: git-workflow
description: Safe local git operations — branch, status, commit; never force-push main.
intent_keywords: [git, commit, branch, merge, push, pull, rebase, stash, diff]
tools:
  - bash_run
  - read_file
---

## Safe defaults

- **`git status`** / **`git diff`** before proposing commits.
- Create feature branches for non-trivial work; avoid committing directly to `main` unless the user asks.
- Write clear, focused commit messages describing *why*.

## Hard limits

- **Never** `git push --force` to `main` or `master`.
- **Never** amend or force-push without explicit user request.
- Do not commit secrets (`.env`, tokens, credentials).

## Typical flow

1. `bash_run git status` — understand dirty state.
2. Make code changes (via approved `edit_file` / `write_file`).
3. `bash_run git add … && git commit -m "…"` only after user intent is clear.

## Anti-patterns

- Large unrelated commits bundling many files.
- Pushing without checking branch and remote state.
