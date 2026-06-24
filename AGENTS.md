# AGENTS.md

Guidance for AI agents working in the **unistar-coworker** repository.

## What this project is

unistar-coworker is a **local GitHub ops secretary** (Rust, ratatui TUI). It:

- Runs scheduled **workflows** (daily triage, release duty, review radar, …) and an interactive **chat** mode.
- Calls **GithubHarness** in-process (`gh` CLI) for GitHub/CI read/write tools; optional **third-party MCP** servers via `mcp.servers[]` ([`src/mcp/`](./src/mcp/)).
- Uses a **local LLM** (Ollama-compatible OpenAI API) for classification, chat planning, and digests.
- **Never** auto-executes mutating actions — GitHub harness tools (`ci_rerun_workflow`, …) and **federated MCP mutating tools** go through the TUI/Web approval queue unless `chat.auto_approve_mutations` (or per-server `approval.mutating: auto` with global auto) is explicitly enabled.

It is **not** a coding agent: no repo editing, no auto-merge, no replacement for GitHub Actions.

Product boundaries and architecture: [README.md](./README.md), [README_CN.md](./README_CN.md), [skill-agent-harness.md](./skill-agent-harness.md).

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

## Skill → Prompt → Harness

Three layers; do not blur responsibilities:

| Layer | Location | Role |
|-------|----------|------|
| **Skill** | `skills/*/SKILL.md` | Reusable technique: triage rules, tone, digest style. No cron, no harness logic. |
| **Prompt** | `prompts/*.md` | Chat system prompt body; frontmatter `skills:` lists default techniques. |
| **Harness** | `src/agent/*.rs`, `src/engine/*.rs` | Deterministic Rust: MCP, store, approvals, token budget, chat/workflow loops. |

Tool names SSOT: [`skills/_base/TOOLS.md`](./skills/_base/TOOLS.md) + [`src/agent/tool_catalog.rs`](./src/agent/tool_catalog.rs).

Prompt assembly: [`src/engine/prompt.rs`](./src/engine/prompt.rs) (`compose_system_prompt`, `load_chat_prompt_bundle`).

---

## Chat harness (most active area)

Entry: [`src/engine/chat.rs`](./src/engine/chat.rs) → [`src/agent/chat_loop.rs`](./src/agent/chat_loop.rs).

| Concern | Where |
|---------|--------|
| Native tool calling (Ollama/OpenAI `tools` / `tool_calls`) | [`src/llm/chat.rs`](./src/llm/chat.rs), [`src/llm/client.rs`](./src/llm/client.rs) |
| Tool schemas exposed to the model | `ToolCatalog::native_tool_definitions_for_session(chat.tool_mode, warmed)` in [`tool_catalog.rs`](./src/agent/tool_catalog.rs) |
| LLM message packing, trimming, token estimate | [`src/agent/context.rs`](./src/agent/context.rs) |
| Full tool args + result/error in context | `format_tool_context_message()` in `context.rs` |
| Harness nudges (missing args, duplicate tool, invalid name) | `tool_catalog.rs` + `push_harness_nudge` in `chat_loop.rs` — **chronological order**, not moved to tail |
| Mutating tool gate | `is_mutating_tool` → approval queue in `chat_loop.rs` / [`src/engine/approvals.rs`](./src/engine/approvals.rs) |
| Session persistence | [`src/store/json.rs`](./src/store/json.rs), [`src/store/sqlite.rs`](./src/store/sqlite.rs) (`data/chat/` when using JSON backend) |

Chat system prompt: [`prompts/chat.md`](./prompts/chat.md) (`chat.prompt` in config; legacy alias `chat.agent`).

Legacy JSON `action: reply | tool | approval` has been removed; chat uses native `tools` / `tool_calls` only.

---

## MCP integration

| Layer | Config / path | Role |
|-------|---------------|------|
| **GitHub** | `github:` in `coworker.yaml` | [`src/github/harness.rs`](./src/github/harness.rs) — in-process `gh`; meta-tools (`tool_list`, `tool_search`, `tool_describe`, `tool_call`) index GitHub + local harness only. |
| **Third-party** | `mcp.servers[]` | [`src/mcp/`](./src/mcp/) — `McpPool`; `transport: stdio` (subprocess) or `http` (Streamable HTTP POST + JSON/SSE). |

- Chat routes federated readonly MCP tools through `execute_readonly_tool` in [`chat_loop.rs`](./src/agent/chat_loop.rs); mutating MCP tools require approval (routing TBD for some paths).
- Lazy mode: when `mcp.servers[]` is non-empty, `tool_list` / `tool_search` / `tool_describe` federate GitHub harness + MCP registry ([`src/mcp/lazy_adapter.rs`](./src/mcp/lazy_adapter.rs)).
- TUI Config tab shows per-server `mcp[id]` status from `AppState.mcp_servers`.

For new **GitHub** tools, extend **GithubHarness** / unistar-mcp catalog — do not duplicate `gh` calls in coworker. For **Slack/filesystem/etc.**, add an MCP server entry under `mcp.servers[]`.

---

## Store, scheduler, TUI

| Area | Path |
|------|------|
| JSON / SQLite store | `src/store/` |
| Cron + workflow dispatch | `src/engine/scheduler.rs`, `src/engine/workflows.rs` |
| TUI (tabs, chat, context panel, approvals) | `src/tui/` |
| CLI entry | `src/main.rs` |

Default store backend is JSON under `./data` (gitignored). SQLite backend and `store migrate` are built in.

---

## Configuration

- Example: [`coworker.example.yaml`](./coworker.example.yaml).
- Loaded from cwd or `~/.config/unistar-coworker/coworker.yaml` (see [`src/config.rs`](./src/config.rs)).
- Key knobs: `repos`, `llm.context_limit` (64K), `chat.max_turns`, `chat.max_tool_calls`, `chat.tool_mode`, `policy.auto_rerun_flaky`, `github:`, `mcp.servers[]`.

---

## Common commands

```sh
cargo check
cargo clippy -- -D warnings    # CI-quality bar; fix all warnings
cargo test
cargo test
cargo run --release            # TUI + scheduler
cargo run --release -- chat --once "Summarize open PRs in acme/widget"
cargo run --release -- run-once --workflow daily-work
```

List skills/workflows: `cargo run --release -- skills list` / `workflows list`.

---

## Conventions for code changes

- **Minimal diff** — match existing style in the file; reuse `tool_catalog`, `context`, `parse` helpers instead of new one-off logic.
- **Rust 2021**, `tokio` async, `thiserror` / `anyhow` for errors.
- **Tests** — unit tests live next to modules (`mod tests`); use `acme/widget` and synthetic JSON; run full `cargo test` before finishing.
- **Comments** — only for non-obvious harness invariants; prefer clear names.
- **No new secrets** in repo; no real session dumps under `data/` in commits.
- **Mutating behavior** — must stay behind approval unless config explicitly opts out.
- **Context budget** — 64K-oriented; history uses ~40% of input; when over budget, older turns batch into one `[earlier context summary]` via LLM (`trim_llm_messages_with_llm`), then incremental trim if needed; harness nudges are never folded into summaries.

When adding a chat tool, update: `TOOLS.md` (if documented), `tool_catalog.rs` `TOOLS` table, and tests.

---

## Related repos

- [unistar-mcp](../unistar-mcp) — Go MCP server (`gh`/`git`); see its `AGENTS.md` for tool design principles.
- MCP PR/CI triage skill: `unistar-mcp/.cursor/skills/pr-ci-triage/SKILL.md` (coworker loads `skills/ci-triage/SKILL.md` for workflows/chat).
