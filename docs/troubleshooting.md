# Troubleshooting

Structured guide for common unistar-coworker problems. Run `unistar-coworker doctor` (or `doctor --json`) first.

## Quick diagnostics

```bash
unistar-coworker --version
unistar-coworker doctor
unistar-coworker doctor --json   # paste into issues — redact secrets
RUST_LOG=debug unistar-coworker serve   # verbose logs
```

## Common issues

| Symptom | Likely cause | What to do |
|---------|--------------|------------|
| Web UI **503** on `/` | `web-ui/dist` missing or binary built without `embed-web-ui` | Use a release tarball, or `cd web-ui && npm run build:fast`; release builds need `cargo build --release --features embed-web-ui` |
| **LLM** check fails in doctor | Ollama not running, wrong `base_url`, or API key env not set | Start Ollama; verify `curl` to `base_url`; `export` vars referenced as `${...}` in config |
| **`gh` / GitHub** failures | Not authenticated | `gh auth status`; `gh auth login` or `export GH_TOKEN=...` |
| **MCP** timeout or disconnect | Subprocess crash, bad `command`/`url`, network | Check TUI/Web Config `mcp[id]` status; logs with `RUST_LOG=debug`; verify MCP server starts manually |
| **Port 8787 in use** | Another `serve` or process | `lsof -i :8787` (macOS/Linux); change `web.bind` or stop the other process |
| **Config parse error** on start | Missing or invalid `coworker.yaml` | `unistar-coworker init` or copy from `coworker.example.yaml` |
| **Approval required** exit code 3 | Headless mutating tool without `--yes` | Use TUI/Web to approve, or `chat --yes` / `rpc --yes` only if you accept the risk |
| **Chat empty / heuristic only** | LLM unreachable | Fix LLM per doctor; chat degrades when the model endpoint is down |

## Web UI

### Blank page or assets 404

- Confirm `web-ui/dist` exists next to the binary (dev) or use `--features embed-web-ui` (release).
- Hard-refresh the browser; check browser console for CSP or 401 errors.

### 401 on API / WebSocket

- Non-localhost bind requires `web.auth_token`.
- Pass `Authorization: Bearer <token>` or load UI with `?token=<token>` once.

### Binding beyond localhost

Default is `127.0.0.1:8787`. If you use `0.0.0.0`:

- Set `web.auth_token` in `coworker.yaml`.
- Prefer Docker port mapping `-p 127.0.0.1:8787:8787` instead of exposing on all interfaces.

## Configuration and secrets

- Never commit `coworker.yaml` with real keys.
- Use `${ENV_VAR}` for `api_key`, MCP `env`, and `headers`.
- If doctor warns about unexpanded `${VAR}`, export the variable in your shell or Docker `-e`.

## Store and disk

```bash
# Check write access
touch ./data/.write-test && rm ./data/.write-test

# Prune old data
unistar-coworker store compact --dry-run
unistar-coworker store compact
```

SQLite backend: ensure `storage.path` parent directory exists and is writable.

## Workflows and cron

- `run-once` blocks third-party MCP by default — set `workflows.mcp_readonly: true` if you need readonly MCP in batch jobs.
- Daemon + TUI: use `--attach` to connect TUI to an existing daemon store.

## Logs

| Source | How |
|--------|-----|
| CLI / serve | `RUST_LOG=info` (default), `debug`, `trace` |
| Audit trail | Under `data/` (JSON or SQLite backend) |
| `gh` errors | Run the suggested `gh` command manually |

## Diagnostic bundle

```bash
unistar-coworker doctor --bundle /tmp/unistar-diagnostic.zip
```

Exports a zip with `doctor.json`, redacted `coworker.yaml`, and `meta.json` (version/platform). No full chat content. Attach to GitHub issues after redacting any remaining secrets.

For quick checks, `unistar-coworker doctor --json` is enough.

## Still stuck?

1. [docs/upgrading.md](./upgrading.md) — if the problem started after an upgrade.
2. [SUPPORT.md](../SUPPORT.md) — open a Bug or Question issue with version, platform, install method, and redacted `doctor --json`.
