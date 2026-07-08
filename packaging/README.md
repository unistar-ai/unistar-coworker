# Packaging

Scripts and templates for building and launching a single-binary deploy workdir.

| Path | Role |
|------|------|
| [`scripts/start-agent.sh`](../scripts/start-agent.sh) | Build web-ui + Rust binary, refresh runtime workdir, launch agent |
| `workdir-template/` | Seed files copied into the runtime workdir (`coworker.yaml`, `AGENTS.md`) |

Runtime output (not in git):

- `../workdir/` — agent cwd (binary, config, `data/`, synced `skills/`)
- `../.data-backup/` — transient backup while rebuilding workdir

Override locations with `START_AGENT_WORKDIR` and `START_AGENT_DATA_BACKUP`.
