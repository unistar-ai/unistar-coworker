# Skills directory

**Technique SSOT** — reusable judgment/style docs composed into agent prompts.

| Skill | Scenario |
|-------|----------|
| `github-ops-tone/` | Secretary tone (`always: true`) |
| `ci-triage/` | PR CI failures — flaky / real / policy |
| `pr-merge/` | Merge blockers, review state |
| `pr-review/` | Diff review, risk scan, CODEOWNERS |
| `my-prs/` | Author's open PRs |
| `pr-hygiene/` | Stale, mega, docs-only PRs |
| `ci-health/` | Main branch / workflow CI stats |
| `external-ci/` | Jenkins, Codecov, non-Actions checks |
| `flaky-tests/` | Flaky ledger, rerun compare |
| `issue-tracker/` | Issues list, search, detail |
| `security-alerts/` | Dependabot / scanning alerts |
| `release-backport/` | Tags, release notes, backports |
| `digest-style/` | Ops digest writing format |
| `oncall-store/` | Local digest, approvals, handoff |
| `_base/TOOLS.md` | Full tool reference (workflows / native mode) |

Each `SKILL.md` follows a consistent shape: **Scope**, **Workflow**, **Output template**, plus YAML `description` / `argument-hint` / `tools:` / intent metadata. Lazy chat lists every skill's `name` and `description` under **Available skills** in the system prompt; the model calls `skill_load` to pull in the full body.

Chat loads skills from **`agents/chat/AGENT.md`** `skills:` list unless `chat.skills` overrides in yaml.

```bash
unistar-coworker agents list
unistar-coworker skills list
```

Task specs: **`agents/<id>/AGENT.md`**. See [skill-agent-harness.md](../skill-agent-harness.md).
