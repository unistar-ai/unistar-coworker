---
name: git-workflow
description: "Safe local git operations — status, diff, branch, commit. Use when the user asks about git state, commits, or branches. Never force-push main."
argument-hint: "What git operation or branch"
intent_keywords: [git, commit, branch, merge, push, pull, rebase, stash, diff]
tools:
  - bash_run
  - read_file
---

# Git Workflow

Inspect before mutating. Clear commits describe *why*; never compromise shared branches without explicit user intent.

## Scope

Use for:
- `git status`, `git diff`, branch creation, staged commits when user intent is clear

Hard limits:
- **Never** `git push --force` to `main` or `master`
- **Never** amend or force-push without explicit user request
- **Never** commit secrets (`.env`, tokens, credentials)

## Workflow

1. **`git status` / `git diff`** — understand dirty state and scope.
2. **Branch** — feature branches for non-trivial work unless user wants direct commits.
3. **Code changes** — via approved `edit_file` / `write_file` (load `code-edit`).
4. **Commit** — follow [docs/COMMITS.md](../../docs/COMMITS.md) (Conventional Commits); `git add … && git commit -m "…"` only when intent is clear.
5. **Push** — check branch and remote; warn before anything destructive.

## Output template

### State
Branch, clean/dirty summary

### Proposed action
Commands you ran or recommend (if any)

### Result
Exit codes and relevant output
