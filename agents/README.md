# Agents

**Chat only.** Interactive assistant spec: `agents/chat/AGENT.md`.

Batch GitHub workflows (daily digest, review radar, issue triage, …) are **built into Rust** — metadata and default skills live in `src/engine/workflow_registry.rs`. Enable them in `coworker.yaml`:

```yaml
workflows:
  daily-work: {}
  review-radar: {}
```

List workflows: `cargo run --release -- workflows list`

Override default technique skills per workflow:

```yaml
workflows:
  daily-work:
    skills:
      - skills/ci-triage/SKILL.md
```

Chat loads skills from `agents/chat/AGENT.md` `skills:` unless `chat.skills` overrides in yaml.

Technique library: **`skills/*`**. See [skill-agent-harness.md](../skill-agent-harness.md).
