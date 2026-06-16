# Skills directory

**Technique SSOT** — reusable judgment/style docs composed into agent prompts.

| Path | Role |
|------|------|
| `ci-triage/` | CI verdict rules (flaky / real / policy) |
| `digest-style/` | Ops digest writing |
| `github-ops-tone/` | Secretary tone, no hallucination |
| `pr-merge/` | Merge blockers interpretation |
| `_base/TOOLS.md` | Shared tool catalog (see `load_skill_with_base` / `read_base_tools`) |

Task specs live in **`agents/<id>/AGENT.md`**. Configure workflows with `agent:` + optional `skills[]` in `coworker.yaml` — see [skill-agent-harness.md](../skill-agent-harness.md).

```bash
unistar-coworker agents list
unistar-coworker skills list
```
