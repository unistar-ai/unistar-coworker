# unistar-coworker

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](./LICENSE)
[![Rust 1.75+](https://img.shields.io/badge/rust-1.75%2B-orange.svg)](https://www.rust-lang.org/)

**Local GitHub ops secretary** — TUI, Web UI, in-process GitHub harness, optional MCP federation, local LLM.

[English](./README.md) · [中文](./README_CN.md)

---

unistar-coworker watches PRs and CI, classifies failures, produces digests, and queues mutating actions for **human approval**. It is an **ops secretary**, not an unconstrained coding agent: no auto-merge, no auto-push fixes, no replacement for GitHub Actions. **Chat** can still use workspace tools (`read_file`, `grep`, `bash_run`, …) for light local work.

GitHub/CI runs **in-process** in Rust ([`GithubHarness`](./src/github/harness.rs) → `gh` CLI). Optional third-party MCP servers (Slack, filesystem, HTTP gateways) mount via `mcp.servers[]` (stdio or Streamable HTTP).

## Table of contents

- [Features](#features)
- [Quick start](#quick-start)
- [Installation](#installation)
- [Usage](#usage)
- [Configuration](#configuration)
- [MCP federation](#mcp-federation)
- [Architecture](#architecture)
- [Development](#development)
- [Contributing](#contributing)
- [Related](#related)
- [License](#license)

## Features

- **Workflows** — `daily-work` (morning triage digest), `review-radar` (CI green, blocked on review); cron, daemon, or one-shot
- **Chat** — Natural-language REPL in TUI, CLI, or Web; LLM plans tool chains across GitHub harness, workspace, and federated MCP
- **GithubHarness** — GitHub/CI tools in-process via `gh`; capped payloads; no MCP subprocess for GitHub
- **MCP federation** — `mcp.servers[]` with stdio + HTTP, lazy discovery, mutating approval, cancel in flight
- **Safety** — Rerun CI, backport, post comment, MCP mutating tools require TUI/Web approval (unless `chat.auto_approve_mutations`)
- **TUI** — Dashboard, PR list, approvals, logs, config, flaky report, release queue, issues, full-screen chat
- **Web UI** — Browser chat (`serve`), sessions, light/dark theme, streaming tool/reasoning cards with source labels
- **Store** — JSON (default) or SQLite for digests, snapshots, flaky ledger, chat sessions, audit log

## Quick start

```bash
cd unistar-coworker
cargo build --release
cp coworker.example.yaml coworker.yaml
# Edit repos, github:, llm.base_url / model

export GH_TOKEN=ghp_...   # or: gh auth login

cargo run --release                              # TUI + cron scheduler
cargo run --release -- serve                     # Web → http://127.0.0.1:8787
cargo run --release -- run-once                  # headless daily-work
cargo run --release -- chat --once "Summarize open PRs in acme/widget"
```

## Installation

| Dependency | Purpose |
|------------|---------|
| **Rust 1.75+** | Build unistar-coworker |
| **`gh` CLI** | GitHub harness; `gh auth login` or `GH_TOKEN` |
| **Ollama / OpenAI-compatible API** (optional) | Local LLM at `llm.base_url` |

```bash
cargo build --release
# Binary: target/release/unistar-coworker
```

[unistar-mcp](../unistar-mcp) is a **standalone** GitHub MCP server (Go). Coworker does **not** require or spawn it at runtime.

## Usage

### TUI (default)

```bash
cargo run --release
```

| Key | Tab |
|-----|-----|
| `0` / `?` | Chat |
| `1` | Dashboard |
| `2` | PR list |
| `3` | Approvals (`y` / `n`) |
| `4` | Logs |
| `5` | Config (github + `mcp[id]` status) |
| `6` | Flaky |
| `7` | Release |
| `8` | Issues |

`Tab` / `Shift+Tab` cycle tabs · `r` run daily-work · `q` quit · `Esc` cancel chat turn.

### Web UI

```bash
cargo run --release -- serve
# Open http://127.0.0.1:8787
```

Streaming chat, tool/reasoning cards with **source** labels (`github` vs `mcp:…`), context pane, approval modal, theme toggle. Static assets: `src/web/static/` (rebuild after UI changes).

### Chat

```bash
cargo run --release -- chat
cargo run --release -- chat --once "Why is #42 CI red in acme/widget?"
cargo run --release -- chat --session <uuid>
```

Mutating GitHub and MCP tools enqueue **Approvals** unless `chat.auto_approve_mutations: true`.

| `chat.tool_mode` | Behavior |
|------------------|----------|
| `auto` (default) | Skill chains, then `tool_search` / `tool_list_category` / `tool_call`; schemas cached per session |
| `lazy` | Same discovery path, minimal upfront context |
| `native` | Full tool schemas exposed up front |

**Workspace tools:** `read_file`, `grep`, `glob`, `edit_file`, `write_file`, `bash_run`, `python_run`, `web_fetch`. File/bash mutating paths use LLM safety review; GitHub/MCP mutating uses human approval.

### Workflows

| Workflow | Summary | Default skills |
|----------|---------|----------------|
| `daily-work` | Morning PR/CI triage → digest + flaky ledger | `ci-triage`, `digest-style` |
| `review-radar` | PRs waiting for review (CI green) | `pr-merge`, `digest-style` |

```bash
cargo run --release -- run-once
cargo run --release -- run-once --workflow review-radar
cargo run --release -- daemon          # cron only, no TUI
cargo run --release -- --attach        # TUI attached to running daemon store
```

### CLI reference

| Command | Description |
|---------|-------------|
| *(default)* | TUI + cron scheduler |
| `serve` | Web UI + API + WebSocket |
| `--attach` | TUI attached to running daemon store |
| `run-once [--workflow ID]` | Headless workflow (default: `daily-work`) |
| `daemon` | Cron only, no TUI |
| `chat [--once MSG] [--session UUID]` | Interactive or one-shot chat |
| `triage-pr --repo O/R --pr N` | Debug triage for one PR |
| `report flaky [--since-days 30]` | Flaky ledger export |
| `store migrate --from json --to sqlite` | Migrate store |
| `skills list` / `workflows list` | Catalog |

### GitHub harness tools

PR: `pr_list_open`, `pr_get_overview`, `pr_get_status`, `pr_get_diff`, `pr_list_changed_files`, `pr_diff_risk_scan`, `pr_create_backport`, …

CI: `ci_analyze_pr_failures`, `ci_get_run_summary`, `ci_get_failed_logs`, `ci_rerun_workflow`, …

Meta: `tool_search`, `tool_list`, `tool_describe`, `tool_call`, `resource_read` (`github://`, `pr://`, `ci://`).

Implemented in [`src/github/harness.rs`](./src/github/harness.rs). Tool names SSOT: [`skills/_base/TOOLS.md`](./skills/_base/TOOLS.md) + [`src/agent/tool_catalog.rs`](./src/agent/tool_catalog.rs).

## Configuration

`coworker.yaml` loads from cwd or `~/.config/unistar-coworker/` (gitignored). Start from [coworker.example.yaml](./coworker.example.yaml).

```yaml
repos:
  - acme/widget

github:
  gh_command: gh
  timeout_secs: 120

llm:
  base_url: http://localhost:11434/v1
  model: your-model
  context_limit: 64000

workflows:
  daily-work: {}
  review-radar: {}

chat:
  workspace: .
  tool_mode: auto   # auto | lazy | native

web:
  bind: 127.0.0.1:8787

theme: dark

policy:
  auto_rerun_flaky: false
```

| Key | Role |
|-----|------|
| `github:` | In-process harness (`gh_command`, `env`, `timeout_secs`, `tool_timeouts`) |
| `mcp.servers[]` | Optional third-party MCP (stdio / http) — see below |
| `chat.prompt` | Chat system prompt file (default `prompts/chat.md`; legacy alias `chat.agent`) |
| `chat.skills` | Override skill list (else from prompt frontmatter `skills:`) |
| `workflows.<id>.skills` | Override default skills per workflow |

## MCP federation

GitHub **always** uses `github:` / `GithubHarness`. External tools (Slack, filesystem, custom HTTP MCP) use `mcp.servers[]`:

| Topic | Behavior |
|-------|----------|
| Transport | `stdio` (subprocess JSON-RPC) or `http` (Streamable HTTP + Bearer headers) |
| Tool names | Flat prefixed names, e.g. `slack_post_message` |
| Discovery | Federated `tool_list` / `tool_search` / `tool_describe` (GitHub + each server section) |
| Mutating | `approval.mutating: required` → same approval queue as `ci_rerun_workflow` (`ApprovalKind::McpTool`) |
| Resources | `resource_read` with `mcp+{server_id}://…` URIs |
| UI | TUI/Web Config: `mcp[id]: ok (N tools)`; tool cards show `mcp:slack · post_message` |
| Reload | Web/TUI **Re-probe** reloads config and reconnects MCP servers |
| Cancel | Chat cancel aborts HTTP requests and kills stdio MCP children |

```yaml
mcp:
  defaults:
    timeout_secs: 120
    startup: on_demand
  servers:
    - id: slack
      transport: stdio
      command: npx
      args: ["-y", "@modelcontextprotocol/server-slack"]
      env:
        SLACK_BOT_TOKEN: ${SLACK_BOT_TOKEN}
      expose:
        prefix: slack_
      approval:
        mutating: required
    - id: ops
      transport: http
      url: http://127.0.0.1:9090/mcp
      headers:
        Authorization: Bearer ${OPS_MCP_TOKEN}
```

Implementation: [`src/mcp/`](./src/mcp/).

## Architecture

```
┌──────────────────────────────────────────────────────────────────┐
│  unistar-coworker (Rust)                                         │
│  TUI / Web → Engine / Scheduler → Prompts + Skills → Store         │
│                    ↓ LLM              ↓ Approvals                │
│  GithubHarness (in-process gh) + McpPool (optional MCP)          │
└──────────────────────────────────────────────────────────────────┘
```

| Entry | Trigger | Orchestration |
|-------|---------|----------------|
| **Workflow** | cron, `run-once`, TUI `r` | Fixed harness loop + skills → digest/store |
| **Chat** | TUI `[0]`, `chat`, Web | `prompts/chat.md` + skills + LLM tool loop |

### Product boundaries

| It is | It is not |
|-------|-----------|
| Reads capped GitHub/CI signals; local LLM assists triage and digests | A replacement for GitHub Actions or a CI runner |
| Ledger, digests, drafts, approval-gated mutating actions | Unapproved auto-merge or repo-wide autonomous edits |
| TUI + Web UI for ops; terminal-first | A hosted SaaS dashboard |
| **Workflow** batch jobs + **Chat** ad-hoc Q&A and light coding | A second `gh` wrapper or required `unistar-mcp` subprocess |

**Non-goals:** no unapproved auto-merge; no full-repo semantic RAG; workflows do not call third-party MCP by default (chat may when configured).

### Skill / Prompt / Harness

| Layer | Location | Role |
|-------|----------|------|
| **Skill** | `skills/*/SKILL.md` | Reusable technique — triage rules, tone, digest format |
| **Prompt** | `prompts/chat.md` | Chat system prompt; `skills:` in frontmatter selects default techniques |
| **Harness** | `src/agent/`, `src/engine/` | Deterministic Rust — scheduler, MCP pool, approvals, chat/workflow loops |

Further detail: [AGENTS.md](./AGENTS.md), [skill-agent-harness.md](./skill-agent-harness.md).

## Development

```bash
cargo check
cargo clippy -- -D warnings
cargo test
```

```
unistar-coworker/
├── prompts/chat.md
├── skills/
├── src/agent/       # chat loop, tool catalog
├── src/engine/      # scheduler, workflows, approvals
├── src/github/      # GithubHarness
├── src/mcp/         # optional MCP federation
├── src/llm/
├── src/tui/
├── src/web/
└── src/store/
```

Crate version: **1.0.0** ([Cargo.toml](./Cargo.toml))

## Contributing

Read [AGENTS.md](./AGENTS.md) for workspace layout, harness conventions, and PR expectations. Skills and prompts live beside the crate; tool names must stay aligned with `TOOLS.md` and `tool_catalog.rs`.

## Related

- [unistar-mcp](../unistar-mcp) — standalone GitHub MCP product (optional; not used by coworker at runtime)
- [README_CN.md](./README_CN.md) — 中文说明

## License

MIT — see [LICENSE](./LICENSE).
