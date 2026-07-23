---
name: gh-cli
description: >-
  Run GitHub operations reliably via the GitHub CLI (`gh`). Use for PRs, issues,
  Actions workflows/runs, releases, secrets, `gh api`, auth, and repo context —
  including when the user gives a GitHub URL, asks to create/review/merge a PR,
  rerun CI, inspect checks, or troubleshoot `gh` failures.
intent_keywords: [github, gh, pull request, github.com, workflow, dependabot, actions]
intent_phrases: [gh pr, gh api, gh run, gh issue, gh repo, gh auth]
intent_bonus_keywords: ["/pull/", "#"]
---

# GitHub CLI (`gh`)

Use `gh` for all GitHub tasks. Run commands via the Shell tool; do not guess flags.

## Pre-flight (every task)

```bash
gh --version
gh auth status
```

Then confirm repo context:

```bash
# In a git checkout
gh repo view --json nameWithOwner,defaultBranchRef -q '.nameWithOwner + " " + .defaultBranchRef.name'

# Outside a repo, or for another repo
export GH_REPO='OWNER/REPO'
# or per command: gh pr list -R OWNER/REPO ...
```

If a command will change remote state, state the impact before running. High-risk ops (merge, delete branch, release publish, secret set, workflow dispatch) need explicit user confirmation unless they already asked for that exact action.

## Core rules

1. **Help before unknown flags** — `gh <cmd> --help` or `gh help <cmd>`. Never invent flags.
2. **Machine-readable output** — prefer `--json` + `-q` / `--template`. Do not parse table text.
3. **Non-interactive** — set `GH_PROMPT_DISABLED=1`; pass `--body-file`, `--title`, `--fill` instead of opening an editor.
4. **Secrets** — never log tokens, secret values, or full `GH_DEBUG` output.
5. **Failures** — keep the raw stderr; check auth, `-R`/`GH_REPO`, permissions, branch protection; then re-read help. Do not invent causes.

`--jq` / `-q` is built into `gh`; a system `jq` binary is not required for `gh` formatting.

## Common workflows

### Inspect PR and CI

```bash
gh pr view "$PR" --json number,title,state,mergeable,reviewDecision,statusCheckRollup,url -R OWNER/REPO
gh pr checks "$PR" -R OWNER/REPO
```

### Create PR (non-interactive)

Gather git context first (`git status`, `git diff`, `git log` vs base). Then:

```bash
gh pr create \
  --title "title" \
  --body "$(cat <<'EOF'
## Summary
- ...

## Test plan
- [ ] ...
EOF
)" \
  --base main \
  -R OWNER/REPO
```

Use `--fill` when title/body can come from commits. Add `--draft` if requested. Omit `--head` when already on the pushed branch.

### Rerun failed CI for a PR

```bash
SHA=$(gh pr view "$PR" -R OWNER/REPO --json headRefOid -q .headRefOid)
gh run list -R OWNER/REPO --commit "$SHA" -L 100 \
  --json databaseId,conclusion,workflowName,status \
  -q '.[] | select(.conclusion == "failure" or .conclusion == "cancelled" or .conclusion == "timed_out") | "\(.workflowName)\t\(.databaseId)"'
# Then per run:
gh run rerun "$RUN_ID" -R OWNER/REPO --failed
gh run watch "$RUN_ID" -R OWNER/REPO
```

Runs older than ~30 days or with 0 jobs cannot be rerun — push a new commit instead.

### Merge PR (confirm first)

```bash
gh pr merge "$PR" --squash --delete-branch -R OWNER/REPO
```

Only one of `--merge`, `--squash`, `--rebase`. Fails if required checks or reviews are missing.

## Command reference

Run `gh <cmd> --help` to list valid `--json` fields for that command.

### Authentication

```bash
gh auth status
gh auth login                    # interactive
gh auth login --web
gh auth refresh --hostname github.com --scopes repo,workflow,read:org,project
gh auth switch                   # when multiple accounts
```

CI / automation:

```bash
export GH_PROMPT_DISABLED=1
export GH_TOKEN="$GITHUB_TOKEN"   # or a PAT; never echo it
```

### Repository context

```bash
gh repo view --json nameWithOwner,defaultBranchRef,url -q .nameWithOwner
gh repo clone OWNER/REPO
gh repo clone OWNER/REPO -- --recurse-submodules
gh repo set-default OWNER/REPO
export GH_REPO='OWNER/REPO'
```

Prefer `-R OWNER/REPO` over `cd` when switching repos.

### Pull requests

```bash
# List / view
gh pr list --state open -L 100 --json number,title,headRefName,baseRefName,author,url -R OWNER/REPO
gh pr list --author '@me' --state open --json number,title,url -R OWNER/REPO
gh pr view "$PR" --json number,title,body,headRefName,baseRefName,author,url,files,statusCheckRollup,mergeable,reviewDecision -R OWNER/REPO

# Branch
gh pr checkout "$PR" -R OWNER/REPO
gh pr update-branch "$PR" -R OWNER/REPO
gh pr update-branch "$PR" --rebase -R OWNER/REPO

# Create
gh pr create --fill --base main -R OWNER/REPO
gh pr create --title "$TITLE" --body-file "$BODY_FILE" --base "$BASE" -R OWNER/REPO
gh pr create --fill --draft -R OWNER/REPO

# Comment / review
gh pr comment "$PR" --body-file "$BODY_FILE" -R OWNER/REPO
gh pr review "$PR" --approve --body 'LGTM' -R OWNER/REPO
gh pr review "$PR" --comment --body-file "$BODY_FILE" -R OWNER/REPO
gh pr review "$PR" --request-changes --body-file "$BODY_FILE" -R OWNER/REPO

# Edit
gh pr edit "$PR" --add-label bug --add-reviewer user -R OWNER/REPO
gh pr edit "$PR" --title 'new title' -R OWNER/REPO

# Merge (one strategy only)
gh pr merge "$PR" --squash --delete-branch -R OWNER/REPO
gh pr merge "$PR" --merge --delete-branch -R OWNER/REPO
gh pr merge "$PR" --rebase --delete-branch -R OWNER/REPO
```

`--head` is only needed for cross-repo PRs or when the branch is not the current branch. `--head` supports `USER:BRANCH` for forks.

### Issues

```bash
gh issue list --state open -L 100 --json number,title,labels,author,url -R OWNER/REPO
gh issue view "$ISSUE" --json number,title,body,comments,labels,url -R OWNER/REPO
gh issue create --title "$TITLE" --body-file "$BODY_FILE" --label bug -R OWNER/REPO
gh issue comment "$ISSUE" --body-file "$BODY_FILE" -R OWNER/REPO
gh issue edit "$ISSUE" --add-label bug --add-assignee user -R OWNER/REPO
```

### GitHub Actions

```bash
gh workflow list -R OWNER/REPO
gh workflow run "$WORKFLOW" --ref "$REF" -R OWNER/REPO
gh workflow run "$WORKFLOW" --ref "$REF" -f key=value -R OWNER/REPO   # dispatch inputs

gh run list -L 20 --json databaseId,status,conclusion,workflowName,headBranch,url -R OWNER/REPO
gh run list --workflow "$WORKFLOW" --commit "$SHA" -L 100 \
  --json databaseId,status,conclusion,workflowName -R OWNER/REPO
gh run view "$RUN_ID" --json status,conclusion,url -R OWNER/REPO
gh run view "$RUN_ID" --log -R OWNER/REPO
gh run watch "$RUN_ID" -R OWNER/REPO

gh run rerun "$RUN_ID" -R OWNER/REPO              # all jobs
gh run rerun "$RUN_ID" --failed -R OWNER/REPO     # failed jobs only

gh pr checks "$PR" -R OWNER/REPO
gh pr checks "$PR" --json name,state,bucket,link -R OWNER/REPO
```

`gh run rerun --job` needs `databaseId` from `gh run view "$RUN_ID" --json jobs -q '.jobs[] | {name, databaseId}'`, not the URL job number.

### Releases

```bash
gh release list -R OWNER/REPO
gh release view "$TAG" -R OWNER/REPO
gh release create "$TAG" --title "$TITLE" --notes-file "$NOTES" --draft -R OWNER/REPO
gh release create "$TAG" "$FILE" --title "$TITLE" --notes-file "$NOTES" -R OWNER/REPO
gh release upload "$TAG" "$FILE" -R OWNER/REPO
gh release upload "$TAG" "$FILE" --clobber -R OWNER/REPO
```

### Secrets

```bash
gh secret list -R OWNER/REPO
echo -n "$VALUE" | gh secret set "$NAME" -R OWNER/REPO
gh secret set "$NAME" -R OWNER/REPO --body "$VALUE"
gh secret set -f .env -R OWNER/REPO    # dotenv file; multiple secrets
```

`gh secret set` has no `--body-file`. Use stdin, `--body`, or `--env-file`. Always confirm before setting.

### gh api

```bash
# GET — {owner}/{repo}/{branch} filled from current repo
gh api repos/{owner}/{repo}/pulls/"$PR" -q .head.ref

# Paginate
gh api --paginate "repos/{owner}/{repo}/issues?state=open&per_page=100" -q '.[].number'

# POST
gh api --method POST repos/{owner}/{repo}/issues \
  -f title="$TITLE" -f body="$BODY"

# GraphQL
gh api graphql \
  -f query='query($owner:String!, $repo:String!){ repository(owner:$owner, name:$repo){ nameWithOwner defaultBranchRef { name } } }' \
  -F owner="$OWNER" -F repo="$REPO" -q .data.repository

# Rate limit
gh api rate_limit -q .resources.core
```

Explicit repo: `gh api /repos/OWNER/REPO/...` or set `GH_REPO`.

### Formatting and debug

```bash
gh pr list --json number,title -q '.[] | "\(.number)\t\(.title)"'
gh pr list --json number,title --template '{{range .}}{{.number}} {{.title}}{{"\n"}}{{end}}'
GH_DEBUG=api gh pr list -R OWNER/REPO   # may leak tokens — do not paste raw logs
```

`gh help formatting` documents `--template` functions (`tablerow`, `pluck`, `timeago`, etc.).

## Troubleshooting

| Symptom | Likely cause | Action |
|---------|--------------|--------|
| `gh: command not found` | CLI not installed | Install from https://cli.github.com/ |
| exit code `4` / `authentication failed` | Not logged in or expired token | `gh auth status`; `gh auth login` or `gh auth refresh`; in CI set `GH_TOKEN` |
| `HTTP 404` on private repo | Wrong repo, no access, or insufficient scopes | Verify `-R OWNER/REPO`, `gh auth status` scopes, org SSO authorization |
| `not a git repository` | CWD is not a checkout | `cd` into repo or `export GH_REPO=OWNER/REPO` / `-R` on every command |
| `could not determine base repository` | No default repo | `gh repo set-default OWNER/REPO` or `-R` |
| `No commits between` | Branch not pushed or identical to base | `git push -u origin HEAD`; confirm diff vs base |
| `Protected branch` / merge blocked | Required checks or reviews | `gh pr checks`; `gh pr view --json reviewDecision,statusCheckRollup` |
| `Resource not accessible by integration` | `GITHUB_TOKEN` / PAT scope too narrow | Add `workflow`, `repo`, or use a PAT with correct permissions |
| `gh run rerun` fails | Run >30 days, fork changed, or 0 jobs | Push new commit; fix workflow YAML if job count is 0 |
| `404` on `gh run rerun --job` | Used URL job number instead of `databaseId` | `gh run view RUN --json jobs -q '.jobs[] \| {name, databaseId}'` |
| Sandbox / permission errors in agent | Shell sandbox blocked network or path | Re-run with `required_permissions: ["all"]` |
| Prompt hangs in automation | Interactive prompt | `export GH_PROMPT_DISABLED=1`; pass all flags explicitly |

**jq confusion:** `gh` ships its own jq engine for `-q` / `--jq`. A missing system `jq` does not block `gh` JSON filtering.

**When rerun is not enough:** YAML errors (0 jobs) require fixing `.github/workflows` and pushing. Fallback: `git commit --allow-empty -m "trigger CI" && git push`.

**Investigation order:** exact command + stderr → `gh auth status` → repo context → command `--help` → `GH_DEBUG=api` (redact before sharing).

## Agent execution notes

- Request `all` or `full_network` shell permissions when `gh` hits auth/network/sandbox errors.
- Exit code `4` → auth required (`gh auth status`, `gh auth login` or `GH_TOKEN`).
- Exit code `8` from `gh pr checks --watch` → checks still pending (not necessarily failure).
- In CI: `export GH_PROMPT_DISABLED=1` and `GH_TOKEN` (or `GITHUB_TOKEN`).

## Report format

**Success:**

```text
Repo: OWNER/REPO
Action: <what>
Result: success
Ref: PR #123 / issue #45 / run 999 / tag v1.0
URL: <link>
Command: <key gh command>
Notes: <optional>
```

**Failure:** include repo, action, exact command, raw error, evidence-based cause (if any), and one verifiable next step.
