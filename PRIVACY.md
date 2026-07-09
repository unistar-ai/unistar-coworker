# Privacy Policy

unistar-coworker is a **local, single-user tool**. It does not phone home with usage analytics or telemetry.

## Summary

| Topic | Behavior |
|-------|----------|
| **Telemetry** | None — no usage data sent to unistar-ai or third-party analytics |
| **Data storage** | Local disk only, under your configured `storage.path` (default `./data`) |
| **LLM requests** | Sent only to endpoints you configure (`llm.base_url`, API keys via env) |
| **GitHub access** | Via your local `gh` CLI and credentials — governed by [GitHub Terms of Service](https://docs.github.com/en/site-policy/github-terms/github-terms-of-service) |
| **Network (optional)** | `upgrade-check` queries **GitHub Releases API** only for version comparison — no personal data transmitted |

## What is stored locally

Default location: `./data` (or `storage.path` in `coworker.yaml`). Typical contents:

| Path / area | Contents |
|-------------|----------|
| Chat sessions | Conversation history, tool call records |
| Audit log | Operational events (may include redacted tool metadata) |
| Digests | Workflow-generated summaries |
| Flaky ledger, snapshots | CI/triage state |

`coworker.yaml` (config) lives outside `data/` — usually the current directory or `~/.config/unistar-coworker/coworker.yaml`. It may reference secrets via `${ENV_VAR}` placeholders; resolved values are not uploaded by unistar-coworker.

## What leaves your machine

Only what **you** configure:

1. **LLM provider** — chat and workflow prompts go to your `base_url` (Ollama, DeepSeek, etc.).
2. **GitHub** — `gh` commands use your authenticated session.
3. **MCP servers** — third-party tools you add under `mcp.servers[]` may contact external services.
4. **GitHub Releases API** — optional version check (`upgrade-check`, planned) fetches public release metadata only.

There is **no** background upload of chat content, config, or diagnostics to project maintainers.

## Deleting your data

To remove all local application data:

```bash
# Stop unistar-coworker first, then:
rm -rf ./data
```

For SQLite backend, delete the configured database file (e.g. `./data/coworker.db`) and related JSON artifacts if any.

To prune without full deletion:

```bash
unistar-coworker store compact --audit-days 90
```

Docker: remove the mounted data volume when destroying the container.

## Session retention

There is no cloud retention policy — **you** control how long data remains on disk. Consider periodic `store compact` and backing up or deleting `data/` before decommissioning a machine.

## Web UI exposure

The Web UI binds to `127.0.0.1` by default. If you expose it beyond localhost, anyone who can reach the port may access chat and approvals unless `web.auth_token` is set. See [SECURITY.md](./SECURITY.md) and [README.md](./README.md#web-ui).

## Questions

See [SUPPORT.md](./SUPPORT.md). This document describes product behavior, not a legal contract.
