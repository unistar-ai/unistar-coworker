# unistar-coworker

Local **GitHub ops secretary** with a TUI — built on [unistar-mcp](https://github.com/unistar-ai/unistar-mcp) and optional local LLM (Ollama/vLLM).

## v0.3

- **Approval → MCP execution** — `y` on Approvals runs `ci_rerun_workflow` or `pr_create_backport`; updates flaky rerun stats
- **Cron scheduler** — `schedule.*` and per-workflow `schedule` in `coworker.yaml` (TUI mode)
- **`release-duty`** — scan merged PRs with `needs-backport` label; queue backport approvals per target branch

See [design.md](./design.md) for the roadmap.

## v0.2 recap

- **`daily-work` pipeline** — PR list → CI triage → LLM/heuristic classify → digest + flaky ledger
- **Skill loading**, JSON/SQLite store, 6-tab TUI, `run-once`

## Requirements

- Rust 1.75+
- [unistar-mcp](https://github.com/unistar-ai/unistar-mcp) on `PATH`
- `gh` authenticated (`GH_TOKEN` or `gh auth login`) — used by MCP and release-duty discovery
- Optional: [Ollama](https://ollama.com) at `llm.base_url`

## Quick start

```sh
cd unistar-coworker
cargo build --release

cd ../unistar-mcp && go build -o unistar-mcp ./cmd
export PATH="$PWD:$PATH"
export GH_TOKEN=...

cd ../unistar-coworker
cp coworker.example.yaml coworker.yaml   # edit repos, llm, etc.
cargo run --release                  # TUI + cron
cargo run --release -- run-once      # daily-work once
cargo run --release -- run-once --workflow release-duty
```

### TUI keys

| Key | Action |
|-----|--------|
| `1`–`6` | Switch tab |
| `r` | Run `daily-work` |
| `R` | Run `release-duty` |
| `j`/`k` | Move selection |
| `y`/`n` | Approve/deny (Approvals — executes MCP on approve) |
| `q` | Quit |

## Config

Copy `coworker.example.yaml` to `coworker.yaml` (gitignored) and edit for your environment:

```sh
cp coworker.example.yaml coworker.yaml
```

```yaml
schedule:
  daily_digest: "0 6 * * *"    # cron → daily-work (if enabled)
  ci_rescan: "0 */4 * * *"

workflows:
  daily-work:
    enabled: true
    skill: skills/daily-work/SKILL.md
  release-duty:
    enabled: false
    skill: skills/release-duty/SKILL.md
    schedule: "0 9 * * 1-5"

release:
  backport_label: needs-backport
  target_branches:
    - release/1.0
  lookback_limit: 30

policy:
  auto_rerun_flaky: false
  auto_backport: false
```

## License

MIT — see [LICENSE](./LICENSE).
