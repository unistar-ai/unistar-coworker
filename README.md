# unistar-coworker

Local **GitHub ops secretary** with a TUI — built on [unistar-mcp](../unistar-mcp) and a local LLM (Ollama/vLLM).

v0.1 ships:

- **TUI** — Dashboard, PR list, Approvals, Logs, Config, Flaky (6 tabs)
- **Workflow `daily-work`** — fetch open PRs via MCP, write digest to JSON store
- **JSON store** (default) — `./data/`; SQLite via `--features sqlite`
- **Headless** — `run-once` for cron/systemd

See [design.md](./design.md) for the full product roadmap.

## Requirements

- Rust 1.75+
- [unistar-mcp](../unistar-mcp) on `PATH` (or set `mcp.command` in config)
- `GH_TOKEN` / `gh auth` for GitHub
- Optional: Ollama at `llm.base_url` (probe only in v0.1; agent uses MCP first)

## Quick start

```sh
cd unistar-coworker
cargo build --release

# edit repos in coworker.yaml
export GH_TOKEN=ghp_...   # or gh auth login

# TUI (default)
cargo run --release

# once, no TUI
cargo run --release -- run-once --workflow daily-work
```

### TUI keys

| Key | Action |
|-----|--------|
| `1`–`6` | Switch tab |
| `r` | Run `daily-work` |
| `j`/`k` | Move selection |
| `y`/`n` | Approve/deny (Approvals tab) |
| `q` | Quit |

## Config

`coworker.yaml` in the project root (or `.coworker/coworker.yaml`).

```yaml
storage:
  backend: json    # or sqlite (cargo build --features sqlite)
  path: ./data

mcp:
  command: unistar-mcp
  args: ["--lazy"]

repos:
  - STARRY-S/unistar-mcp
```

Build unistar-mcp from the sibling repo:

```sh
cd ../unistar-mcp && go build -o unistar-mcp ./cmd
export PATH="$PWD:$PATH"
```

## SQLite

```sh
cargo build --release --features sqlite
```

Set `storage.backend: sqlite` and `storage.path: ./coworker.db`.

## License

MIT
