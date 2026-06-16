# AGENTS.md

Guidance for AI agents working in the **unistar-coworker** repository.

## What this project is

unistar-coworker is a **local GitHub ops secretary** (Rust, ratatui TUI). It:

- Runs scheduled **workflows** (daily triage, release duty, review radar, …) and an interactive **chat** mode.
- Calls **[unistar-mcp](https://github.com/unistar-ai/unistar-mcp)** over stdio MCP (`--lazy` by default) for GitHub/CI read/write tools.
- Uses a **local LLM** (Ollama-compatible OpenAI API) for classification, chat planning, and digests.
- **Never** auto-executes mutating actions — `ci_rerun_workflow`, `pr_create_backport`, `pr_post_comment` go through the TUI approval queue unless `chat.auto_approve_mutations` is explicitly enabled.

It is **not** a coding agent: no repo editing, no auto-merge, no replacement for GitHub Actions.

Product boundaries and architecture: [README.md](./README.md), [design.md](./design.md), [skill-agent-harness.md](./skill-agent-harness.md).

---

## Sensitive information (required when writing code)

**Strip or redact secrets and identifying production data** in every change — source, tests, docs, fixtures, logs, and commit messages.

| Do not commit or paste | Use instead |
|------------------------|-------------|
| `GH_TOKEN`, `ghp_*`, `github_pat_*`, or any API key | `GH_TOKEN` mentioned only as an env var name; tests use no token |
| Real `owner/repo` names from your org | `acme/widget`, `owner/repo` (match existing tests) |
| Real PR/issue numbers tied to production | Fictional numbers (`19263`, `42`, …) |
| User home paths, internal hostnames, VPN URLs | `/path/to/unistar-mcp`, `http://localhost:11434/v1` |
| Contents of `coworker.yaml`, `data/`, `digests/` | [coworker.example.yaml](./coworker.example.yaml) for shape only |
| Chat session exports, audit logs, approval IDs from a live run | Synthetic UUIDs and placeholder text |

`coworker.yaml` and `data/` are **gitignored** — never add them to the repo. If you touch example config, keep models/paths generic.

When adding tests or debug output, prefer **synthetic MCP payloads** over copying tool results from a real session.

---

## Skill → Agent → Harness

Three layers; do not blur responsibilities:

| Layer | Location | Role |
|-------|----------|------|
| **Skill** | `skills/*/SKILL.md` | Reusable technique: triage rules, tone, digest style. No cron, no harness logic. |
| **Agent** | `agents/*/AGENT.md` | Task spec: goals, output format, tool strategy; references `skills[]`. |
| **Harness** | `src/agent/*.rs`, `src/engine/*.rs` | Deterministic Rust: MCP, store, approvals, token budget, chat/workflow loops. |

Tool names SSOT: [`skills/_base/TOOLS.md`](./skills/_base/TOOLS.md) + `config.chat.preferred_tools` + [`src/agent/tool_catalog.rs`](./src/agent/tool_catalog.rs).

Prompt assembly: [`src/engine/prompt.rs`](./src/engine/prompt.rs) (`compose_system_prompt`, `load_chat_prompt_bundle`).

---

## Chat harness (most active area)

Entry: [`src/engine/chat.rs`](./src/engine/chat.rs) → [`src/agent/chat_loop.rs`](./src/agent/chat_loop.rs).

| Concern | Where |
|---------|--------|
| Native tool calling (Ollama/OpenAI `tools` / `tool_calls`) | [`src/llm/chat.rs`](./src/llm/chat.rs), [`src/llm/client.rs`](./src/llm/client.rs) |
| Tool schemas exposed to the model | `ToolCatalog::native_tool_definitions()` in [`tool_catalog.rs`](./src/agent/tool_catalog.rs) |
| LLM message packing, trimming, token estimate | [`src/agent/context.rs`](./src/agent/context.rs) |
| Full tool args + result/error in context | `format_tool_context_message()` in `context.rs` |
| Harness nudges (missing args, duplicate tool, invalid name) | `tool_catalog.rs` + `push_harness_nudge` in `chat_loop.rs` — **chronological order**, not moved to tail |
| Mutating tool gate | `is_mutating_tool` → approval queue in `chat_loop.rs` / [`src/engine/approvals.rs`](./src/engine/approvals.rs) |
| Session persistence | [`src/store/json.rs`](./src/store/json.rs), [`src/store/sqlite.rs`](./src/store/sqlite.rs) (`data/chat/` when using JSON backend) |

Chat agent spec: [`agents/chat/AGENT.md`](./agents/chat/AGENT.md).

Legacy JSON `action: reply | tool | approval` parsing remains in `llm/chat.rs` for classify workflow and salvage tests; **chat path uses native tool calls**.

---

## MCP integration

- Config: `mcp.command`, `mcp.args` (typically `["--lazy"]`) in `coworker.yaml`.
- Implementation: [`src/mcp/subprocess.rs`](./src/mcp/subprocess.rs) — stdio JSON-RPC to unistar-mcp; no shell.
- Lazy mode: model sees `tool_list` / `tool_describe` / `tool_call` meta-tools plus chat `preferred_tools` via native schemas.

For MCP server behavior or new GitHub tools, change **unistar-mcp**, not duplicate `gh` calls in coworker.

---

## Store, scheduler, TUI

| Area | Path |
|------|------|
| JSON / SQLite store | `src/store/` |
| Cron + workflow dispatch | `src/engine/scheduler.rs`, `src/engine/workflows.rs` |
| TUI (tabs, chat, context panel, approvals) | `src/tui/` |
| CLI entry | `src/main.rs` |

Default store backend is JSON under `./data` (gitignored). SQLite: `cargo test --features sqlite`, `store migrate`.

---

## Configuration

- Example: [`coworker.example.yaml`](./coworker.example.yaml).
- Loaded from cwd or `~/.config/unistar-coworker/coworker.yaml` (see [`src/config.rs`](./src/config.rs)).
- Key knobs: `repos`, `llm.context_limit` (64K), `chat.max_turns`, `chat.max_tool_calls`, `chat.preferred_tools`, `policy.auto_rerun_flaky`.

---

## Common commands

```sh
cargo check
cargo clippy -- -D warnings    # CI-quality bar; fix all warnings
cargo test
cargo test --features sqlite
cargo run --release            # TUI + scheduler
cargo run --release -- chat --once "Summarize open PRs in acme/widget"
cargo run --release -- run-once --workflow daily-work
```

List agents/skills: `cargo run --release -- agents list` / `skills list`.

---

## Conventions for code changes

- **Minimal diff** — match existing style in the file; reuse `tool_catalog`, `context`, `parse` helpers instead of new one-off logic.
- **Rust 2021**, `tokio` async, `thiserror` / `anyhow` for errors.
- **Tests** — unit tests live next to modules (`mod tests`); use `acme/widget` and synthetic JSON; run full `cargo test` before finishing.
- **Comments** — only for non-obvious harness invariants; prefer clear names.
- **No new secrets** in repo; no real session dumps under `data/` in commits.
- **Mutating behavior** — must stay behind approval unless config explicitly opts out.
- **Context budget** — 64K-oriented; trimming in `trim_llm_messages` compresses old tool turns; harness nudges are protected from aggressive trim (`is_harness_nudge_content`).

When adding a chat tool to the whitelist, update: `TOOLS.md` (if documented), `tool_catalog.rs` `TOOLS` table, tests, and optionally `chat.preferred_tools` in example yaml.

---

## Related repos

- [unistar-mcp](https://github.com/unistar-ai/unistar-mcp) — Go MCP server (`gh`/`git`); see its `AGENTS.md` for tool design principles.
- MCP PR/CI triage skill: `unistar-mcp/.cursor/skills/pr-ci-triage/SKILL.md` (coworker loads `skills/ci-triage/SKILL.md` for workflows/chat).
