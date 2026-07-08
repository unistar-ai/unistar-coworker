# GitHub ops pack (optional integration)

GitHub and CI behavior is **not** the product identity — it is an optional **domain pack** loaded via skills, `repos:` + `github:` config, and built-in batch workflows.

- **Harness:** in-process `GithubHarness` (`gh` CLI) — no `unistar-mcp` subprocess required for GitHub.
- **Chat:** load skills on demand (router + `chat.skills` + prompt frontmatter).
- **Batch:** `daily-work`, `review-radar` (see [`docs/workflows.md`](../../docs/workflows.md)).

Enable in `coworker.yaml`:

```yaml
github:
  gh_command: gh
repos:
  - owner/repo
workflows:
  daily-work:
    schedule: "0 9 * * 1-5"
```

Workspace-only setups can omit all of the above; `doctor` warns instead of failing when `repos:` is empty.

---

## Skills catalog

| Skill | Use when |
|-------|----------|
| [`github-ops-tone`](../github-ops-tone/SKILL.md) | User works in GitHub/CI context; concise ops tone |
| [`ci-triage`](../ci-triage/SKILL.md) | PR or run is red — classify failure |
| [`ci-health`](../ci-health/SKILL.md) | Default-branch / workflow health rollups |
| [`external-ci`](../external-ci/SKILL.md) | Non–GitHub Actions checks (Jenkins, Codecov, …) |
| [`flaky-tests`](../flaky-tests/SKILL.md) | Flake ledger, reruns, compare runs |
| [`pr-review`](../pr-review/SKILL.md) | Review a PR diff and comments |
| [`pr-merge`](../pr-merge/SKILL.md) | Merge readiness, review queue |
| [`pr-hygiene`](../pr-hygiene/SKILL.md) | Stale/large PR hygiene |
| [`my-prs`](../my-prs/SKILL.md) | Author’s open PRs |
| [`release-backport`](../release-backport/SKILL.md) | Tags, release notes, backport candidates |
| [`security-alerts`](../security-alerts/SKILL.md) | Dependabot / security alerts |
| [`issue-tracker`](../issue-tracker/SKILL.md) | Issues search and triage |
| [`digest-style`](../digest-style/SKILL.md) | Morning digest formatting (workflows) |
| [`oncall-store`](../oncall-store/SKILL.md) | On-call handoff from local store |
| [`git-workflow`](../git-workflow/SKILL.md) | Local git operations in workspace |
| [`gh-cli`](../gh-cli/SKILL.md) | When to prefer `gh` via bash vs harness tools |
| [`repo-explore`](../repo-explore/SKILL.md) | Repo metadata and exploration |
| [`debug`](../debug/SKILL.md) | Deep CI/debug sessions |
| [`test-run`](../test-run/SKILL.md) | Local test execution patterns |

**Not in this pack** (general agent):

| Skill | Role |
|-------|------|
| [`general-agent-tone`](../general-agent-tone/SKILL.md) | Always-on default tone |
| [`code-edit`](../code-edit/SKILL.md) | Workspace edit workflow |
| [`web-fetch`](../web-fetch/SKILL.md) | HTTP/HTML fetch (non-GitHub) |

---

## Built-in workflows

| ID | Default skills | Purpose |
|----|----------------|---------|
| `daily-work` | `ci-triage`, `digest-style` | Multi-repo triage → digest + flaky ledger |
| `review-radar` | `pr-merge`, `digest-style` | CI-green PRs blocked on review |

Override skills per workflow:

```yaml
workflows:
  daily-work:
    skills: [ci-triage, digest-style, flaky-tests]
```

List registry: `unistar-coworker workflows list`.

---

## Tool reference

Full harness/MCP vocabulary: [`skills/_base/TOOLS.md`](../_base/TOOLS.md).

Author new GitHub skills: [`skills/_base/SKILL_TEMPLATE.md`](../_base/SKILL_TEMPLATE.md).
