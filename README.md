# unistar-coworker

Local **GitHub ops secretary** with a TUI — built on [unistar-mcp](https://github.com/unistar-ai/unistar-mcp) and optional local LLM (Ollama/vLLM).

## v0.2 — first usable release

- **Skill loading** — parse `skills/<workflow>/SKILL.md` frontmatter + body; inject body into LLM classification
- **`daily-work` pipeline**:
  1. `pr_list_open` per repo
  2. For CI-failing PRs: `pr_get_status` → `ci_analyze_pr_failures` → `ci_get_failed_logs`
  3. **LLM or heuristic** classify flaky vs real bug
  4. Record **`flaky_incidents`** + rollup; queue **rerun approvals** in TUI
  5. Write structured **Daily Digest** (attention / flaky / ok)
- **TUI** — 6 tabs; `r` runs workflow; `y`/`n` on approvals
- **Store** — JSON (default) or SQLite (`--features sqlite`)
- **Headless** — `run-once --workflow daily-work`

See [design.md](./design.md) for the roadmap.

## Requirements

- Rust 1.75+
- [unistar-mcp](https://github.com/unistar-ai/unistar-mcp) on `PATH`
- `GH_TOKEN` or `gh auth login`
- Optional: [Ollama](https://ollama.com) at `llm.base_url` (falls back to log heuristics if offline)

## Quick start

```sh
cd unistar-coworker
cargo build --release

# build MCP sibling
cd ../unistar-mcp && go build -o unistar-mcp ./cmd
export PATH="$PWD:$PATH"
export GH_TOKEN=...

cd ../unistar-coworker
# edit repos in coworker.yaml

cargo run --release                  # TUI
cargo run --release -- run-once      # daily-work once
```

### TUI keys

| Key | Action |
|-----|--------|
| `1`–`6` | Switch tab |
| `r` | Run `daily-work` |
| `j`/`k` | Move selection |
| `y`/`n` | Approve/deny rerun (Approvals tab) |
| `q` | Quit |

## Config (`coworker.yaml`)

```yaml
mcp:
  command: unistar-mcp
  args: ["--lazy"]

llm:
  base_url: http://localhost:11434/v1
  model: gemma3:27b   # or any Ollama OpenAI-compatible model

storage:
  backend: json
  path: ./data

workflows:
  daily-work:
    enabled: true
    skill: skills/daily-work/SKILL.md

repos:
  - unistar-ai/unistar-mcp

policy:
  auto_rerun_flaky: false   # flaky reruns go to Approvals tab
  max_tool_calls_per_pr: 5
```

## License

MIT — see [LICENSE](./LICENSE).
