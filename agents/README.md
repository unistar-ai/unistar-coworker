# Agents directory

**Task SSOT** — one `AGENT.md` per workflow or chat mode.

| Agent | Harness entry |
|-------|----------------|
| `chat/` | Interactive JSON loop (`run_chat_turn`) |
| `daily-work/` | `AgentLoop::run_daily_work` |
| `merge-health/` | `run_merge_health` |
| … | `src/agent/loop.rs` dispatch |

Optional frontmatter:

```yaml
---
name: daily-work
description: Morning triage digest
skills: [ci-triage, digest-style]
---
```

Configure in `coworker.yaml`:

```yaml
workflows:
  daily-work:
    agent: agents/daily-work/AGENT.md
    skills:
      - skills/ci-triage/SKILL.md
```

```bash
unistar-coworker agents list
```

See [skill-agent-harness.md](../skill-agent-harness.md).
