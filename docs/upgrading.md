# Upgrading unistar-coworker

This guide covers moving between releases. The authoritative version is `unistar-coworker --version` (matches `Cargo.toml` workspace version).

## Before you upgrade

1. **Read [CHANGELOG.md](../CHANGELOG.md)** — especially `### Breaking` / `### API` sections for your target version.
2. **Back up local data** — copy `data/` (or your `storage.path`) and `coworker.yaml`.
3. **Run doctor** — `unistar-coworker doctor` should pass on the current version.

```bash
cp -a data data.backup.$(date +%Y%m%d)
cp coworker.yaml coworker.yaml.backup
```

## tar.gz release (Linux x86_64, macOS arm64)

Official builds are published on [GitHub Releases](https://github.com/unistar-ai/unistar-coworker/releases).

1. Download the tarball for your platform: `unistar-coworker-<version>-<triple>.tar.gz`.
2. Verify checksum (`.sha256` file alongside the archive).
3. Stop any running `serve` or TUI instance.
4. Extract the archive to your install directory (or replace only the binary).
5. **Keep** your existing `coworker.yaml` and `data/` — do not overwrite them with the template inside the tarball unless you intend a fresh install.
6. Run `unistar-coworker doctor` and `unistar-coworker --version`.
7. Start `serve` or TUI as usual.

From source:

```bash
git fetch --tags
git checkout v2.1.0   # or target tag
(cd web-ui && npm ci && npm run build:fast)
cargo build --release --features embed-web-ui
```

## Docker

Images are published to GHCR on version tags. See [docs/docker.md](./docker.md).

```bash
docker pull ghcr.io/unistar-ai/unistar-coworker:2.1.0
# Stop old container; start new image with the same volume mounts
# Always map: -p 127.0.0.1:8787:8787
```

Config and data live on mounted volumes (`/config`, `/data`) — upgrading the image tag does not delete them.

## Configuration migrations

- `coworker.yaml` is generally **forward compatible** across minor releases.
- Optional `config_version` field (default `1`) reserves room for future migrations; `unistar-coworker doctor` reports config warnings after upgrade.
- If a release notes a manual config change, edit `coworker.yaml` before restarting.

## Checking for updates

```bash
unistar-coworker upgrade-check
unistar-coworker upgrade-check --json
```

- Offline or rate-limited runs warn and exit `0`. Web UI Config tab also shows when an update is available after `serve` starts.

Also see [CHANGELOG.md](../CHANGELOG.md) and [GitHub Releases](https://github.com/unistar-ai/unistar-coworker/releases).

## Cross-major upgrades

Major versions (e.g. 2.x → 3.x) may remove **Stable** RPC operations or change defaults. Always:

1. Read the full CHANGELOG for the major release.
2. Run integration tests for any `rpc` scripts.
3. Re-run `doctor` and spot-check Web UI + one chat turn (`chat --once "hello"`).

## Rollback

1. Stop the new binary/container.
2. Restore the previous binary or image tag.
3. Restore `data/` and `coworker.yaml` from backup if the new version migrated or wrote incompatible state.

## Getting help

See [troubleshooting.md](./troubleshooting.md) and [SUPPORT.md](../SUPPORT.md).
