# unistar-coworker

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](./LICENSE)
[![Rust 1.75+](https://img.shields.io/badge/rust-1.75%2B-orange.svg)](https://www.rust-lang.org/)
[![CI](https://github.com/unistar-ai/unistar-coworker/actions/workflows/ci.yml/badge.svg)](./.github/workflows/ci.yml)

**A local GitHub ops secretary** — terminal-first TUI, browser Web UI, in-process GitHub harness, optional MCP federation, and a local LLM. It watches PRs and CI, classifies failures, produces digests, and queues every mutating action behind **human approval**.

[English](./README.md) · [中文](./README_CN.md)

---

## Overview

`unistar-coworker` is **not** an unconstrained coding agent and **not** a replacement for GitHub Actions. It is an ops secretary that:

- Runs scheduled **workflows** (`daily-work` triage, `review-radar`) and an interactive **chat** mode for ad-hoc Q&A and light local work.
- Calls GitHub/CI **in-process** in Rust via the [`GithubHarness`](./crates/core/src/github/harness.rs) → `gh` CLI — no MCP subprocess for GitHub.
- Mounts optional **third-party MCP** servers (Slack, filesystem, HTTP gateways) through `mcp.servers[]` (stdio or Streamable HTTP).
- Uses a **local LLM** (Ollama / OpenAI-compatible) for classification, chat planning, and digests.
- **Never** auto-executes mutating actions — rerun CI, backport, post comment, and MCP mutating tools all go through a TUI/Web approval queue unless `chat.auto_approve_mutations` is explicitly enabled.

Chat can still use workspace tools (`read_file`, `grep`, `bash_run`, …) for light local coding; file/bash mutating paths go through LLM safety review, while GitHub/MCP mutating paths require human approval.

---

## Table of contents

- [unistar-coworker](#unistar-coworker)
  - [Overview](#overview)
  - [Table of contents](#table-of-contents)
  - [Features](#features)
  - [Quick start](#quick-start)
  - [Requirements](#requirements)
  - [Usage](#usage)
    - [TUI](#tui)
    - [Web UI](#web-ui)
    - [Chat](#chat)
    - [Workflows](#workflows)
    - [CLI reference](#cli-reference)
    - [GitHub harness tools](#github-harness-tools)
  - [Configuration](#configuration)
  - [Storage](#storage)
  - [MCP federation](#mcp-federation)
  - [Architecture](#architecture)
    - [Product boundaries](#product-boundaries)
    - [Skill / Prompt / Harness](#skill--prompt--harness)
    - [Development](#development)
    - [Fast compile (dev loop)](#fast-compile-dev-loop)
    - [Web UI development (HMR)](#web-ui-development-hmr)
    - [Web UI E2E (Playwright)](#web-ui-e2e-playwright)
    - [Feature flags](#feature-flags)
  - [Project layout](#project-layout)
  - [Contributing](#contributing)
  - [Related](#related)
  - [License](#license)

---

## Features

| Area | Capability |
|------|------------|
| **Workflows** | `daily-work` (morning PR/CI triage → digest + flaky ledger), `review-radar` (CI-green PRs blocked on review); cron, daemon, or one-shot |
| **Chat** | Natural-language REPL in TUI, CLI, or Web; LLM plans tool chains across GitHub harness, workspace, and federated MCP |
| **GithubHarness** | GitHub/CI tools in-process via `gh`; capped payloads; no MCP subprocess for GitHub |
| **MCP federation** | `mcp.servers[]` with stdio + HTTP, lazy discovery, mutating approval, per-server skills, cancel in flight |
| **Safety** | Rerun CI, backport, post comment, MCP mutating tools require TUI/Web approval (unless `chat.auto_approve_mutations` or per-server `approval.mutating: auto`) |
| **TUI** | Dashboard, PR list, approvals, logs, config, flaky report, release queue, issues, full-screen chat |
| **Web UI** | Browser chat (`serve`), sessions, light/dark theme, streaming tool/reasoning cards, LLM profile switcher, branch regenerate, approval modal, Markdown/JSONL export |
| **Scripting** | `doctor`, `init`, `rpc` (JSONL stdin/stdout), `export session`, shell completions, stable exit codes (`0/2/3/4`) |
| **Ops** | `SIGHUP` / `POST /api/reload` hot-reload config, skills, prompts, MCP; `GET /api/doctor` health JSON |
| **Sessions** | Pi-style message tree — regenerate / branch from any assistant reply; export active branch as JSONL or HTML |
| **LLM** | Named `llm:` profile map + runtime switch (Web Config, RPC `switch_profile`, sidecar `coworker.llm-profile`) |
| **Safety** | Secret redaction in UI, approvals, and CLI output; mutating tools still require human approval |
| **Store** | JSON (default) or SQLite for digests, snapshots, flaky ledger, chat sessions, audit log; `store migrate` and `store compact` commands |

---

## Quick start

```bash
cd unistar-coworker
unistar-coworker init --repos acme/widget --llm-url http://localhost:11434/v1
# Or: cp coworker.example.yaml coworker.yaml and edit manually

export GH_TOKEN=ghp_...   # or: gh auth login

unistar-coworker doctor          # config / gh / LLM / MCP / store health

# Frontend: build once (dev serves from disk; release embeds into the binary)
(cd web-ui && npm install && npm run build:fast)

cargo build --release --features embed-web-ui

./target/release/unistar-coworker                                   # TUI + cron scheduler
./target/release/unistar-coworker serve                             # Web → http://127.0.0.1:8787
./target/release/unistar-coworker run-once                          # headless daily-work
./target/release/unistar-coworker chat --once "Summarize open PRs in acme/widget" --json
```

---

## Requirements

| Dependency | Purpose |
|------------|---------|
| **Rust 1.75+** (toolchain `stable`) | Build `unistar-coworker` |
| **`gh` CLI** | GitHub harness; authenticate via `gh auth login` or `GH_TOKEN` |
| **Ollama / OpenAI-compatible API** (optional) | Local LLM at `llm.base_url`; chat/triage degrade to heuristics when offline |

```bash
# Release / deploy (single binary with embedded Web UI)
cargo build --release --features embed-web-ui
# Binary: target/release/unistar-coworker

# Dev (faster — Web UI read from web-ui/dist/ at runtime; see Development)
cargo build
# Binary: target/debug/unistar-coworker
```

> [unistar-mcp](../unistar-mcp) is a **standalone** GitHub MCP server (Go). Coworker does **not** require or spawn it at runtime — GitHub always goes through the in-process `GithubHarness`.

---

## Usage

### TUI

The default command launches the terminal UI with the cron scheduler attached.

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

`Tab` / `Shift+Tab` cycle tabs · `r` run daily-work · `q` quit · `Esc` cancel the current chat turn.

### Web UI

```bash
cargo run --release -- serve
# Open http://127.0.0.1:8787
```

The Web UI is a **React 18 SPA** (Vite + Tailwind + Radix UI + zustand). It provides streaming chat with live tool/reasoning cards, a context pane, an approval modal, **LLM profile switching** (Config tab), **branch regenerate** on any assistant message, theme toggle, and transcript export. Source lives in `web-ui/`.

**How assets are served:**

| Build | Command | Web UI delivery |
|-------|---------|-----------------|
| **Dev** (default) | `cargo build` / `cargo run -- serve` | Reads `web-ui/dist/` from disk at runtime — Rust-only edits do not re-embed JS bundles |
| **Release / CI** | `cargo build --release --features embed-web-ui` | `build.rs` embeds `web-ui/dist/` via `include_str!` / `include_bytes!` for a single-binary deploy |

`build.rs` does **not** run `npm` — the frontend build is owned by the developer, CI, or `start-agent.sh` (`npm run build:fast`). With `embed-web-ui`, the generated manifest is content-gated so the crate only recompiles when bundled assets actually change. Without `embed-web-ui`, run `npm run build:fast` once so `serve` can find `web-ui/dist/`; if `dist/` is missing the React routes return 503 (legacy UI may still be available).

**Hot reload** (no process restart): send `SIGHUP` to a running `serve` / `tui` / `daemon`, or `POST /api/reload` — reloads `coworker.yaml`, skills, prompts, and MCP connections.

**Health API:** `GET /api/doctor` returns the same JSON report as `unistar-coworker doctor --json` (config, `gh`, LLM, MCP, store).

**Development with HMR:**

```bash
# Terminal 1: Rust backend
cargo run -- serve

# Terminal 2: Vite dev server (hot reload, proxies /api and /ws to :8787)
cd web-ui && npm install && npm run dev
# Open http://localhost:5173
```

**Security model.** The Web UI is intended for **trusted local use** on your machine. Keep `web.bind` at the default `127.0.0.1:8787` so chat, approvals, and workflows are not exposed on the LAN.

When you must bind beyond localhost (e.g. `0.0.0.0`), set `web.auth_token`:

- **Static assets** (`/`, `/assets/*`) remain public so browsers can load them as subresources. They contain no secrets — only UI shape.
- **Sensitive routes** require authentication: all `/api/*` (except `/api/health`) and the `/ws` WebSocket upgrade.
- Two auth methods are accepted:
  - `Authorization: Bearer <token>` header (preferred for API clients, curl).
  - `?token=<token>` query parameter (for `new WebSocket()`, which cannot set headers).
- The browser UI reads `?token=` on first load, stores it in `sessionStorage`, strips it from the URL, and injects it into every fetch and WebSocket request automatically.
- `/api/health` stays unauthenticated so external health probes keep working without credentials.
- A strict **Content-Security-Policy** header is attached to every response: `script-src 'self'` (no inline scripts), `object-src 'none'`, `frame-ancestors 'none'`, `connect-src 'self' ws: wss:`.

> The `?token=` query form can appear in server logs or browser history; for stronger security prefer a reverse proxy that injects an auth cookie. Leave `auth_token` unset for normal localhost development.

### Chat

```bash
cargo run --release -- chat                                       # interactive REPL
cargo run --release -- chat --once "Why is #42 CI red in acme/widget?"
cargo run --release -- chat --once "Summarize open PRs" --json    # script-friendly JSON on stdout
cargo run --release -- chat --session <uuid>                      # resume a session
cargo run --release -- chat --list-sessions --json
cargo run --release -- chat --title "read the README"             # name a new session
```

The CLI chat REPL is built on **rustyline**: line editing (←/→/Home/End), ↑/↓ input history persisted to `coworker-cli-history.txt`, a colored `you·<short-id>>` prompt (auto-disabled when stdout is not a TTY), and **streamed** assistant replies — partial tokens render live as the LLM generates, instead of waiting for the whole turn.

| REPL keys / commands | Behavior |
|----------------------|----------|
| `Ctrl-C` (during a turn) | Cancel the in-flight turn (mirrors TUI `Esc`) — does not exit |
| `Ctrl-C` (at the prompt) | Clear the current input line |
| `Ctrl-D` / `/quit` | Exit the REPL |
| `/help` | List slash commands |
| `/sessions` | List recent sessions (`*` marks the current one) |
| `/new` | Start a fresh session on the next message |
| `/resume <id>` | Resume an existing session |
| `/clear` | Clear the screen |

`chat --once` streams to stdout (so `$(...)` capture works) and prints tool progress to stderr; with `--json` it emits `{ok, session_id, assistant, tool_calls, awaiting_approval}` and uses stable exit codes:

| Code | Meaning |
|------|---------|
| `0` | Success |
| `1` | General error |
| `2` | Config / environment (`doctor` failure, bad config) |
| `3` | Approval required (headless without `--yes`) |
| `4` | Timeout (`--timeout`) |

Mutating GitHub and MCP tools enqueue **Approvals** unless `chat.auto_approve_mutations: true`.

| `chat.tool_mode` | Behavior |
|------------------|----------|
| `auto` (default) | Skill chains, then `tool_search` / `tool_list_category` / `tool_call`; schemas cached per session |
| `lazy` | Same discovery path, minimal upfront context |
| `native` | Full tool schemas exposed up front |

**Workspace tools:** `read_file`, `grep`, `glob`, `edit_file`, `write_file`, `bash_run`, `python_run`, `web_fetch`. File/bash mutating paths use LLM safety review; GitHub/MCP mutating uses human approval.

**Resilience knobs** (optional):

- `chat.llm_step_timeout_secs` — wall clock per LLM step (0 = unlimited).
- `chat.reasoning_only_warn_secs` — stop the stream when only reasoning grows and no visible content arrives (0 = off). Avoids 90s waits on reasoning-only models.

### Workflows

| Workflow | Summary | Default skills |
|----------|---------|----------------|
| `daily-work` | Morning PR/CI triage → digest + flaky ledger | `ci-triage`, `digest-style` |
| `review-radar` | PRs waiting for review (CI green) | `pr-merge`, `digest-style` |

```bash
cargo run --release -- run-once
cargo run --release -- run-once --workflow review-radar --json   # structured JSON on stdout
cargo run --release -- daemon                  # cron only, no TUI (Ctrl-C / SIGTERM to stop)
cargo run --release -- daemon --pid-file ./coworker.pid
cargo run --release -- --attach                # TUI attached to a running daemon's store
```

Batch workflows **block third-party MCP by default**; set `workflows.mcp_readonly: true` (global) or `workflows.<id>.mcp_readonly: true` (per-workflow) to allow readonly MCP only. Mutating MCP stays chat-only.

### CLI reference

**Global flags** (before the subcommand): `--config <PATH>` override config file (skip discover); `-v` / `--verbose` (`-v` debug, `-vv` trace); `-q` / `--quiet` (warn); `--plain` disable ANSI color; `--attach` attach to a daemon store only (TUI / `serve` / `daemon`).

| Command | Description |
|---------|-------------|
| *(default)* / `tui` | TUI + cron scheduler |
| `serve [--bind ADDR]` | Web UI + API + WebSocket |
| `run-once [--workflow ID] [--json] [--timeout SECS]` | Headless workflow (default: `daily-work`) |
| `daemon [--pid-file FILE]` | Cron only, no TUI; graceful SIGINT/SIGTERM, writes+removes pid file |
| `chat [--once MSG] [--session UUID] [--list-sessions] [--title NAME] [--json] [--yes] [--timeout SECS]` | Interactive or one-shot chat |
| `rpc [--session UUID] [--yes] [--timeout SECS]` | JSONL machine protocol on stdin/stdout — see [docs/RPC.md](./docs/RPC.md) |
| `doctor [--json]` | Health check: config, `gh`, LLM, MCP servers, store |
| `init [--force] [--path FILE] [--repos A,B] [--llm-url URL]` | Create starter `coworker.yaml` |
| `export session <UUID> [--format jsonl\|html] [--output FILE]` | Export active chat branch (JSONL or HTML) |
| `completions {bash,zsh,fish,powershell}` | Shell completion scripts |
| `triage-pr --repo O/R --pr N [--json] [--timeout SECS]` | Debug triage for a single PR |
| `report oncall [--json]` | On-call handoff pack from local store (no MCP) |
| `report ci [--since-days 7] [--json]` | CI efficiency report (requires MCP) |
| `store migrate --from json --to sqlite --source DIR --dest FILE` | Migrate store backend |
| `store compact [--audit-days 90] [--digest-keep 30] [--workflow-runs-days 30] [--dry-run]` | Prune old audit entries, digests, workflow runs |
| `skills list [--json]` / `workflows list [--json]` | Print catalog |

`--json` is available on script-oriented commands for machine-readable stdout; human progress stays on stderr. `store compact --dry-run` reports what *would* be pruned without deleting.

**Shell completions** (after building):

```bash
unistar-coworker completions zsh > "${fpath[1]}/_unistar-coworker"   # zsh
unistar-coworker completions bash >> ~/.bashrc                      # bash — then source
```

**RPC mode** drives the agent from scripts or other services without the Web UI:

```bash
printf '%s\n' '{"op":"chat","message":"triage PR #42"}' | unistar-coworker rpc --yes
```

See [docs/RPC.md](./docs/RPC.md) for the full protocol (`chat`, `get_state`, `cancel`, `switch_profile`).

### GitHub harness tools

PR: `pr_list_open`, `pr_get_overview`, `pr_get_status`, `pr_get_diff`, `pr_list_changed_files`, `pr_diff_risk_scan`, `pr_create_backport`, …

CI: `ci_analyze_pr_failures`, `ci_get_run_summary`, `ci_get_failed_logs`, `ci_rerun_workflow`, …

Meta: `tool_search`, `tool_list`, `tool_describe`, `tool_call`, `resource_read` (`github://`, `pr://`, `ci://`).

Implemented in [`crates/core/src/github/harness.rs`](./crates/core/src/github/harness.rs). Tool names SSOT: [`skills/_base/TOOLS.md`](./skills/_base/TOOLS.md) + [`crates/core/src/agent/tool_catalog.rs`](./crates/core/src/agent/tool_catalog.rs).

---

## Configuration

`coworker.yaml` loads from the current directory or `~/.config/unistar-coworker/coworker.yaml` (both gitignored). Start from [coworker.example.yaml](./coworker.example.yaml).

```yaml
repos:
  - acme/widget

# Named LLM presets — switch at runtime (Web Config / RPC / sidecar coworker.llm-profile)
llm_profile: default
llm:
  default:
    base_url: http://localhost:11434/v1
    model: your-model
    context_limit: 64000
  fast:
    base_url: http://localhost:11434/v1
    model: qwen2.5:7b
    context_limit: 32000

github:
  gh_command: gh
  timeout_secs: 120
  # tool_timeouts:
  #   ci_get_failed_logs: 180

workflows:
  # mcp_readonly: false   # global default — batch workflows do not call third-party MCP
  daily-work: {}
  review-radar: {}

chat:
  workspace: .
  tool_mode: auto        # auto | lazy | native
  # compaction: ops      # ops | code | generic — or { strategy: ops, summary_model: fast }
  # llm_step_timeout_secs: 180
  # reasoning_only_warn_secs: 30
  # bash: { timeout_secs: 30, max_output_chars: 16000 }
  # python: { timeout_secs: 30, max_output_chars: 16000, command: python3 }
  # web_fetch:
  #   timeout_secs: 30
  #   max_content_chars: 32000
  #   allow_localhost: true
  #   browser_timeout_secs: 60
  #   chromium_path: /Applications/Google Chrome.app/Contents/MacOS/Google Chrome

web:
  bind: 127.0.0.1:8787
  # auth_token: your-secret   # required for non-localhost bind; protects static assets, /api/*, /ws

theme: dark   # dark | light | none (Web treats none as dark)

policy:
  auto_rerun_flaky: false
```

| Key | Role |
|-----|------|
| `github:` | In-process harness (`gh_command`, `env`, `timeout_secs`, `tool_timeouts`) |
| `llm` / `llm_profile` | Named LLM endpoint map + active preset; runtime switch persists to `coworker.llm-profile` |
| `chat.compaction` | Context compression strategy (`ops` / `code` / `generic`); optional `summary_model` profile |
| `mcp.servers[]` | Optional third-party MCP (stdio / http) — see [MCP federation](#mcp-federation) |
| `chat.prompt` | Chat system prompt file (default `prompts/chat.md`, embedded at build time; custom paths load from disk) |
| `chat.skills` | Override skill list (otherwise from prompt frontmatter `skills:`) |
| `chat.tool_mode` | Tool discovery strategy — see [Chat](#chat) |
| `chat.auto_approve_mutations` | Skip the approval queue for mutating tools (default `false`) |
| `web.bind` | `serve` listen address (default `127.0.0.1:8787`) |
| `web.auth_token` | Bearer token for static assets, `/api/*`, and `/ws` when binding beyond localhost |
| `workflows.<id>.skills` | Override default skills per workflow |
| `workflows.mcp_readonly` | Global default: allow readonly third-party MCP in batch workflows (default `false`) |
| `workflows.<id>.mcp_readonly` | Per-workflow override; mutating MCP stays chat-only |
| `policy.auto_rerun_flaky` | Auto-rerun flaky CI (default `false`; requires approval gate otherwise) |

---

## Storage

The default backend is JSON under `./data` (gitignored). For long-running `serve` / `daemon` deployments or many chat sessions, prefer **SQLite** — single-file, better concurrent reads, and large histories:

```yaml
storage:
  backend: sqlite
  path: ./data/coworker.db
```

Migrate an existing JSON store:

```bash
cargo run --release -- store migrate --from json --to sqlite \
  --source ./data --dest ./data/coworker.db
```

Prune old data to keep the store compact:

```bash
cargo run --release -- store compact            # defaults: audit 90d, keep 30 digests, workflow runs 30d
cargo run --release -- store compact --audit-days 180 --digest-keep 60
cargo run --release -- store compact --dry-run  # preview what would be pruned, delete nothing
```

---

## MCP federation

GitHub **always** uses the in-process `GithubHarness`. External tools (Slack, filesystem, custom HTTP MCP) use `mcp.servers[]`:

| Topic | Behavior |
|-------|----------|
| Transport | `stdio` (subprocess JSON-RPC) or `http` (Streamable HTTP + Bearer headers) |
| Tool names | Flat prefixed names, e.g. `slack_post_message` |
| Discovery | Federated `tool_list` / `tool_search` / `tool_describe` (GitHub + each server section) |
| Mutating | `approval.mutating: required` → same approval queue as `ci_rerun_workflow` (`ApprovalKind::McpTool`) |
| Resources | `resource_read` with `mcp+{server_id}://…` URIs |
| UI | TUI/Web Config: per-server `connected`, `tool_count`, `last_rpc_ms`, `last_error`; tool cards show `mcp:slack · post_message` |
| Reload | `SIGHUP` or `POST /api/reload` reloads config, skills, prompts, MCP; Web/TUI **Re-probe** also reconnects MCP |
| Per-server skills | `skills: [name]` on a server auto-loads those technique skills when its tools are warmed in chat |
| Cancel | Chat cancel aborts HTTP requests and kills stdio MCP children |
| Workflows | Batch workflows block third-party MCP by default; opt in via `workflows.mcp_readonly: true` or per-workflow `mcp_readonly: true` (readonly only) |

```yaml
mcp:
  defaults:
    timeout_secs: 120
    startup: on_demand      # on_demand | eager | disabled
  servers:
    - id: slack
      enabled: true
      transport: stdio
      command: npx
      args: ["-y", "@modelcontextprotocol/server-slack"]
      env:
        SLACK_BOT_TOKEN: ${SLACK_BOT_TOKEN}
      expose:
        prefix: slack_
      approval:
        mutating: required
        tools: [post_message]
      skills: [slack-ops]
    - id: ops
      enabled: true
      transport: http
      url: http://127.0.0.1:9090/mcp
      headers:
        Authorization: Bearer ${OPS_MCP_TOKEN}
```

Implementation: [`crates/core/src/mcp/`](./crates/core/src/mcp/).

---

## Architecture

```
┌──────────────────────────────────────────────────────────────────┐
│  unistar-coworker (Rust)                                         │
│  TUI / Web → Engine / Scheduler → Prompts + Skills → Store        │
│                    ↓ LLM              ↓ Approvals                 │
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
| **Workflow** batch jobs + **Chat** ad-hoc Q&A and light coding | A second `gh` wrapper or a required `unistar-mcp` subprocess |

**Non-goals:** no unapproved auto-merge; no full-repo semantic RAG; workflows do not call third-party MCP by default (chat may when configured).

### Skill / Prompt / Harness

Three layers; do not blur responsibilities:

| Layer | Location | Role |
|-------|----------|------|
| **Skill** | `skills/*/SKILL.md` | Reusable technique — triage rules, tone, digest format. No cron, no harness logic. |
| **Prompt** | `prompts/chat.md` | Chat system prompt body; `skills:` frontmatter selects default techniques. Embedded at build time. |
| **Harness** | `crates/core/src/agent/`, `crates/core/src/engine/` | Deterministic Rust — scheduler, MCP pool, approvals, token budget, chat/workflow loops |

Tool names SSOT: [`skills/_base/TOOLS.md`](./skills/_base/TOOLS.md) + [`crates/core/src/agent/tool_catalog.rs`](./crates/core/src/agent/tool_catalog.rs).

Further detail: [AGENTS.md](./AGENTS.md).

---

## Development

```bash
# Fast Rust iteration (no frontend embed)
cargo check
cargo check -p coworker-tui    # TUI-only loop
cargo clippy --workspace -- -D warnings
cargo test --workspace

# CI / release bar (embeds web-ui/dist/)
cargo clippy --workspace --features embed-web-ui -- -D warnings
cargo test --workspace --features embed-web-ui

cargo test --workspace --no-default-features   # slim build without headless Chromium
cargo fmt --check
```

### Fast compile (dev loop)

Default `cargo build` and `cargo check` **omit** the `embed-web-ui` feature. The React UI is served from `web-ui/dist/` at runtime ([`crates/web/src/ui.rs`](./crates/web/src/ui.rs)), so editing Rust code does not force a full crate recompile when frontend assets change.

The Rust code is split into a **Cargo workspace** (`crates/core`, `crates/tui`, `crates/web`, `crates/cli`, `crates/unistar-coworker`). When a dependency crate is unchanged, `cargo check -p coworker-tui` (or `-p coworker-web`, etc.) skips recompiling unrelated layers.

```bash
# One-time (or when web-ui sources change)
cd web-ui && npm install && npm run build:fast

# Backend — incremental, no JS embed
cargo run -- serve          # http://127.0.0.1:8787
```

Optional local speedups live in [`.cargo/config.toml`](./.cargo/config.toml): `debug = 1`, incremental builds, and commented hooks for `sccache` / `mold` if installed.

**Release / deploy** (single binary, embedded UI — same as CI and `start-agent.sh`):

```bash
(cd web-ui && npm run build:fast)
cargo build --release --features embed-web-ui
```

`cargo build` never depends on Node. Install Node only to build the React UI:

```bash
brew install node          # macOS
cd web-ui && npm install   # first time
```

### Web UI development (HMR)

```bash
# Terminal 1: Rust backend
cargo run -- serve

# Terminal 2: Vite dev server (hot reload, proxies /api and /ws to :8787)
cd web-ui && npm run dev
# Open http://localhost:5173
```

CI (`.github/workflows/ci.yml`) runs `cargo fmt --check`, `cargo clippy --workspace` / `cargo test --workspace` with `--features embed-web-ui` (after building `web-ui/dist/`), a `--no-default-features` build/test job, and an **optional** Playwright smoke job (`continue-on-error: true`).

### Web UI E2E (Playwright)

Smoke tests live in [`web-e2e/`](./web-e2e/) — page load, theme toggle, approvals tab. They start a real `unistar-coworker serve` instance via Playwright `webServer` and a minimal temp `coworker.yaml`.

```bash
(cd web-ui && npm run build:fast)              # dist/ required for e2e
cargo build --features embed-web-ui            # binary at target/debug/unistar-coworker
cd web-e2e
npm install
npx playwright install chromium                # first time: download Chromium
npm test
```

Optional: `UNISTAR_BIN=/path/to/unistar-coworker npm test` if the binary is elsewhere; `E2E_PORT=18787` to change the test bind port.

### Feature flags

| Feature | Default | Purpose |
|---------|---------|---------|
| `web-browser` | on | Headless Chromium for `web_fetch` browser mode (pulls in `chromiumoxide`). Disable with `--no-default-features` for a slimmer build that falls back to HTTP-only `web_fetch`. |
| `embed-web-ui` | off | Embed `web-ui/dist/` into the binary at compile time (`include_str!`). Enable for release builds, CI, and `start-agent.sh`; omit for faster local `cargo check` / `cargo build` (UI served from disk). |

A vendored `chromiumoxide` patch lives under `vendor/chromiumoxide/` for CDP schema drift resilience.

The Web UI (`web-ui/`) requires Node 18+ and is built with `npm run build:fast` (owned by the developer / CI / `start-agent.sh`, **not** by `build.rs`). With `embed-web-ui`, the resulting bundle is compiled into the binary; without it, `serve` reads `web-ui/dist/` at runtime.

---

## Project layout

```
unistar-coworker/
├── .cargo/config.toml       # Dev profile: debug=1, incremental; optional sccache/mold
├── Cargo.toml               # Workspace root
├── crates/
│   ├── core/                # config, store, llm, github, mcp, agent, engine, app
│   ├── tui/                 # ratatui terminal UI
│   ├── web/                 # axum Web server + embed-web-ui build.rs
│   ├── cli/                 # clap subcommands, terminal helpers, chat REPL
│   └── unistar-coworker/    # Thin binary (`main.rs` → `coworker_cli::run`)
├── docs/RPC.md              # JSONL rpc mode protocol
├── prompts/chat.md          # Chat system prompt (embedded at build time)
├── skills/                  # Technique skills (SKILL.md) + _base/TOOLS.md SSOT
├── web-ui/                  # React 18 SPA (Vite + Tailwind + Radix + zustand)
│   ├── src/                 # TypeScript source
│   └── dist/                # vite build output (gitignored, generated)
├── vendor/chromiumoxide/    # Patched CDP dependency
├── web-e2e/                 # Playwright smoke tests
├── coworker.example.yaml    # Config template
└── Cargo.lock
```

Crate version: **1.0.0** (workspace `[workspace.package]` in [Cargo.toml](./Cargo.toml)).

---

## Contributing

Read [AGENTS.md](./AGENTS.md) for the workspace layout, harness conventions, sensitive-data rules, and PR expectations. Skills and prompts live beside the crate; tool names must stay aligned between `TOOLS.md` and `tool_catalog.rs`.

Conventions:

- **Minimal diff** — match existing style; reuse `tool_catalog`, `context`, `parse` helpers.
- **Rust 2021**, `tokio` async, `thiserror` / `anyhow` for errors.
- **Tests** — unit tests live next to modules (`mod tests`); use `acme/widget` and synthetic JSON; run `cargo test` before finishing.
- **No new secrets** in repo; `coworker.yaml` and `data/` are gitignored.
- **Mutating behavior** stays behind approval unless config explicitly opts out.
- When adding a chat tool, update `TOOLS.md`, `tool_catalog.rs`, and tests together.

---

## Related

- [docs/RPC.md](./docs/RPC.md) — JSONL `rpc` mode for scripts and integrations.
- [unistar-mcp](../unistar-mcp) — standalone GitHub MCP server (Go); optional, not used by coworker at runtime.
- [README_CN.md](./README_CN.md) — 中文说明.

---

## License

MIT — see [LICENSE](./LICENSE).
