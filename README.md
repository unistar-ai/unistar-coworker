# unistar-coworker

**v1.0** — Local GitHub ops secretary with a terminal UI, MCP tool bridge, and optional local LLM.

unistar-coworker watches PRs and CI, classifies failures, produces digests, and queues mutating actions for human approval. It is **not** a coding agent: no auto-merge, no auto-push fixes, no replacement for GitHub Actions.

Built on [unistar-mcp](../unistar-mcp) (Go + `gh`), with a Rust harness, ratatui TUI, and Ollama/vLLM chat.

---

## What you get

| Area | Capabilities |
|------|----------------|
| **Workflows** | 18 scheduled agents — daily triage, release duty, main-guard, review radar, flaky govern, issue/security digests, light review, and more |
| **Chat** | Natural-language REPL/TUI; LLM plans MCP tool chains; live reasoning/tool status; LLM context panel |
| **TUI** | Dashboard, PR list, approvals, logs, config, flaky report, release queue, issues, full-screen chat |
| **Safety** | Rerun CI, backport, post comment — executed only after TUI approval |
| **Store** | JSON (default) or SQLite — digests, snapshots, flaky ledger, chat sessions, audit log |
| **Scheduler** | Cron in TUI or headless `daemon` mode |

---

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│  unistar-coworker (Rust)                                    │
│  TUI/CLI → Engine/Scheduler → Agent+Skills → Store          │
│                    ↓ LLM (Ollama)    ↓ Approvals            │
└──────────────────────────┬──────────────────────────────────┘
                           │ stdio MCP (--lazy)
               ┌───────────▼───────────┐
               │ unistar-mcp → gh API  │
               └───────────────────────┘
```

Scheduler or chat → skill/agent spec → MCP tools (capped payloads) → optional LLM classify → digest/store → TUI or Markdown export.

Details: [design.md](./design.md) · harness notes: [skill-agent-harness.md](./skill-agent-harness.md)

---

## Requirements

| Dependency | Purpose |
|------------|---------|
| **Rust 1.75+** | Build unistar-coworker |
| **[unistar-mcp](../unistar-mcp)** | GitHub/CI MCP server (`mcp.command` in config) |
| **`gh` CLI** | Used by MCP; `gh auth login` or `GH_TOKEN` |
| **Ollama** (optional) | Local LLM at `llm.base_url`; some paths work offline with heuristics |

---

## Quick start

```bash
# 1. Build MCP (sibling checkout or your fork)
cd ../unistar-mcp && go build -o unistar-mcp .

# 2. Build coworker
cd ../unistar-coworker && cargo build --release

# 3. Configure
cp coworker.example.yaml coworker.yaml
# Edit repos, mcp.command, llm.model

export GH_TOKEN=ghp_...   # or: gh auth login
export PATH="/path/to/unistar-mcp:$PATH"

# 4. Run
cargo run --release                              # TUI + cron
cargo run --release -- run-once                  # one-shot daily-work
cargo run --release -- chat --once "Summarize open PRs in acme/widget"
```

---

## Configuration

`coworker.yaml` is loaded from the cwd or `~/.config/unistar-coworker/` (gitignored). Start from [coworker.example.yaml](./coworker.example.yaml).

```yaml
repos:
  - acme/widget

mcp:
  command: /path/to/unistar-mcp
  args: ["--lazy"]
  lazy: true              # false = direct tools/call (skip meta-tool hop)
  timeout_secs: 120       # default MCP RPC timeout
  tool_timeouts:
    ci_get_failed_logs: 180   # per-tool overrides
  env: {}

llm:
  base_url: http://localhost:11434/v1
  model: your-model
  context_limit: 64000
  think: true
  max_thinking_tokens: 512

chat:
  enabled: true
  agent: agents/chat/AGENT.md
  max_duration_secs: 900

policy:
  auto_rerun_flaky: false
  auto_backport: false
```

Enable workflows under `workflows.<id>.enabled`. List agents/skills:

```bash
unistar-coworker agents list
unistar-coworker skills list
```

| Workflow | Summary |
|----------|---------|
| `daily-work` | Morning PR/CI triage → digest + flaky ledger |
| `release-duty` | Backport queue from merged PRs |
| `main-guard` | Default-branch CI streak alerts |
| `review-radar` | PRs waiting for review (CI green) |
| `flaky-govern` | Flaky test rollup |
| `my-pr-brief` | Your open PRs across repos |
| + 12 more | issue/security digests, merge health, light review, … |

---

## Chat

Interactive layer on the same MCP + approval stack. The chat agent does **not** invoke other agents — only configured skills and MCP tools.

```bash
cargo run --release -- chat
cargo run --release -- chat --once "Why is #42 CI red in acme/widget?"
cargo run --release -- chat --list-sessions
```

Press **`0`** or **`?`** in the TUI for full-screen chat.

**During a turn** (tail area, not mixed into history until done):

| Phase | UI |
|-------|-----|
| Waiting for JSON | `waiting for model` |
| Thinking stream | `reasoning` |
| Tool JSON | `preparing tool` |
| MCP call | `running <tool>` |
| Reply | `streaming reply` → `▌ AI` row |

**Slash commands:** `/help` `/clear` `/new` `/sessions` `/session <id>` `/export [path]` `/approve` `/deny`

**Keys:** `Enter` send · `Shift+Enter` newline · `Esc` cancel turn · `\` toggle LLM Context panel · `o` expand last tool output

Mutating tools (`ci_rerun_workflow`, `pr_create_backport`, `pr_post_comment`) enqueue **Approvals** — never run directly from chat. When pending, a **centered approval popup** appears: **click Approve/Deny**, or use **←/→** / **Tab** to choose and **Enter** (or **y**/**n**). You can also use `/approve` `/deny` or tab **3** for the full queue. Set `chat.auto_approve_mutations: true` to skip the popup and run mutations immediately (default **off**).

Default read tools are documented in [`skills/_base/TOOLS.md`](./skills/_base/TOOLS.md) (workflows). Chat default **`tool_mode: auto`**: skill chains → `tool_search` / `tool_list_category` → `tool_call` (+ optional `resource_read`); session caches `tool_list` and warms native schemas for tools used. Set `native` for full schemas. See [base-tool-plan.md](./base-tool-plan.md).

---

## CLI

| Command | Description |
|---------|-------------|
| *(default)* | TUI + cron scheduler |
| `--attach` | TUI attached to running daemon store |
| `run-once [--workflow ID]` | Headless workflow (default: `daily-work`) |
| `daemon` | Cron only, no TUI |
| `chat [--once MSG] [--session UUID]` | Interactive or one-shot chat |
| `triage-pr --repo O/R --pr N` | Debug triage for one PR |
| `report flaky [--since-days 30]` | Flaky ledger export |
| `store migrate --from json --to sqlite` | Migrate store (json ↔ sqlite) |

---

## TUI tabs

| Key | Tab |
|-----|-----|
| `0` / `?` | Chat |
| `1` | Dashboard |
| `2` | PR list |
| `3` | **Approvals** (`y` / `n`) |
| `4` | Logs |
| `5` | Config |
| `6` | Flaky |
| `7` | Release |
| `8` | Issues |

`Tab` / `Shift+Tab` cycle tabs. `r` daily-work · `R` release-duty · `q` quit. Theme: `tui.theme: dark | light | none`; optional `tui.accent: "#RRGGBB"`. `--attach` polls the daemon store every 2s.

---

## MCP tools

PR: `pr_list_open`, `pr_get_overview`, `pr_get_status`, `pr_get_diff`, `pr_list_changed_files`, `pr_create_backport`, …

CI: `ci_analyze_pr_failures`, `ci_get_run_summary`, `ci_get_failed_logs`, `ci_rerun_workflow`, …

With `--lazy`, MCP exposes `tool_search`, `tool_list_category`, `tool_list`, `tool_describe`, `tool_call`. Coworker chat adds `resource_read` for MCP resources. See [unistar-mcp/docs/TOOLS.md](../unistar-mcp/docs/TOOLS.md).

---

## Development

```bash
cargo check
cargo clippy -- -D warnings
cargo test
cargo test
```

```
unistar-coworker/
├── agents/      # AGENT.md per workflow / chat
├── skills/      # SKILL.md techniques + _base/TOOLS.md
├── src/agent/   # chat loop, harness, triage, tool catalog
├── src/engine/  # scheduler, prompts, approvals
├── src/llm/     # Ollama client, classify, chat JSON
├── src/tui/     # ratatui UI
└── src/store/   # JSON + SQLite
```

---

## v1.0 highlights

- **Stable chat harness** — tool arg auto-fill, harness nudges that survive context trimming, duplicate-tool guards, approval gate for mutating MCP calls
- **64K context** — compaction, reasoning compression, live context panel with token budget
- **18 workflows** + cron scheduler and daemon attach mode
- **Flaky CI triage** — heuristic + LLM classify, rerun approval queue
- **TUI polish** — Markdown chat, virtual scroll, dark/light/none theme, streaming status tail

Crate version: **1.0.0** ([Cargo.toml](./Cargo.toml))

---

## Related

- [unistar-mcp](../unistar-mcp) — MCP server wrapping `gh`
- [design.md](./design.md) — product spec and boundaries

## License

MIT — see [LICENSE](./LICENSE).
