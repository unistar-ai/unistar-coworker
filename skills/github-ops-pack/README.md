# GitHub ops pack (optional integration)

GitHub and CI behavior is **not** the product identity — it is an optional **domain pack** loaded via skills, `github:` config, and chat tools.

- **Harness:** in-process `GithubHarness` (`gh` CLI) — no external GitHub MCP required.
- **Chat:** load skills on demand (`ci-triage`, `my-prs`, …) and call harness tools with explicit `repo` (or PR links in the message).

Enable in `coworker.yaml`:

```yaml
github:
  gh_command: gh
```

Workspace-only setups can omit all of the above.

---

## Skills catalog

| Skill | Use when |
|-------|----------|
| [`github-ops-tone`](../github-ops-tone/SKILL.md) | User works in GitHub/CI context |
| [`ci-triage`](../ci-triage/SKILL.md) | PR or run is red |
| [`ci-health`](../ci-health/SKILL.md) | Default-branch / workflow health |
| [`external-ci`](../external-ci/SKILL.md) | Non–GitHub Actions checks |
| [`flaky-tests`](../flaky-tests/SKILL.md) | Flake ledger, reruns |
| [`pr-review`](../pr-review/SKILL.md) | Review a PR diff |
| [`pr-merge`](../pr-merge/SKILL.md) | Merge readiness |
| [`pr-hygiene`](../pr-hygiene/SKILL.md) | Stale/large PR hygiene |
| [`my-prs`](../my-prs/SKILL.md) | Author's open PRs |
| [`release-backport`](../release-backport/SKILL.md) | Tags, release notes, backports |
| [`security-alerts`](../security-alerts/SKILL.md) | Dependabot alerts |
| [`issue-tracker`](../issue-tracker/SKILL.md) | Issues search |
| [`git-workflow`](../git-workflow/SKILL.md) | Local git in workspace |
| [`gh-cli`](../gh-cli/SKILL.md) | When to use `gh` via bash vs harness |
| [`repo-explore`](../repo-explore/SKILL.md) | Repo metadata |
| [`debug`](../debug/SKILL.md) | Deep CI/debug |
| [`test-run`](../test-run/SKILL.md) | Local test patterns |

**General agent (not GitHub-specific):** `general-agent-tone`, `code-edit`, `web-fetch`.

---

## Tool reference

[`skills/_base/TOOLS.md`](../_base/TOOLS.md) · author skills: [`SKILL_TEMPLATE.md`](../_base/SKILL_TEMPLATE.md)
