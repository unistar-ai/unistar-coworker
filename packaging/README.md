# Packaging

Scripts and templates for building and launching a single-binary deploy workdir.

| Path | Role |
|------|------|
| [`scripts/package.sh`](../scripts/package.sh) | Build web-ui + Rust binary, refresh runtime workdir (packaging only) |
| `workdir-template/` | Seed files copied into the runtime workdir (`coworker.yaml`, `AGENTS.md`) |

Runtime output (not in git):

- `../workdir/` — agent cwd (binary, config, `data/`, synced `skills/`)
- `../.data-backup/` — transient backup while rebuilding workdir

Override locations with `START_AGENT_WORKDIR` and `START_AGENT_DATA_BACKUP`.

### GitHub Releases

Tag push (`v*`) runs [`.github/workflows/release.yml`](../.github/workflows/release.yml) — release archives mirror the layout above (binary + `skills/` + `template/`).
