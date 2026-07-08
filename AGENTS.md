# AGENTS.md

Guidance for AI agents working in the **unistar-coworker** repository.

## What this project is

unistar-coworker is a **local GitHub ops secretary** (Rust, ratatui TUI). It:

- Runs scheduled **workflows** (daily triage, release duty, review radar, …) and an interactive **chat** mode.
- Calls **GithubHarness** in-process (`gh` CLI) for GitHub/CI read/write tools; optional **third-party MCP** servers via `mcp.servers[]` ([`crates/core/src/mcp/`](./crates/core/src/mcp/)).
- Uses a **local LLM** (Ollama-compatible OpenAI API) for classification, chat planning, and digests.
- **Never** auto-executes mutating actions — GitHub harness tools (`ci_rerun_workflow`, …) and **federated MCP mutating tools** go through the TUI/Web approval queue unless `chat.auto_approve_mutations` (or per-server `approval.mutating: auto` with global auto) is explicitly enabled.

It is **not** a coding agent: no repo editing, no auto-merge, no replacement for GitHub Actions.

Product boundaries and architecture: [README.md](./README.md), [README_CN.md](./README_CN.md).

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
| **Harness** | `crates/core/src/agent/*.rs`, `crates/core/src/engine/*.rs` | Deterministic Rust: MCP, store, approvals, token budget, chat/workflow loops. |

Tool names SSOT: [`skills/_base/TOOLS.md`](./skills/_base/TOOLS.md) + [`crates/core/src/agent/tool_catalog.rs`](./crates/core/src/agent/tool_catalog.rs).

Prompt assembly: [`crates/core/src/engine/prompt.rs`](./crates/core/src/engine/prompt.rs) (`compose_system_prompt`, `load_chat_prompt_bundle`).

---

## Chat harness (most active area)

Entry: [`crates/core/src/engine/chat.rs`](./crates/core/src/engine/chat.rs) → [`crates/core/src/agent/chat_loop.rs`](./crates/core/src/agent/chat_loop.rs).

| Concern | Where |
|---------|--------|
| Native tool calling (Ollama/OpenAI `tools` / `tool_calls`) | [`crates/core/src/llm/chat.rs`](./crates/core/src/llm/chat.rs), [`crates/core/src/llm/client.rs`](./crates/core/src/llm/client.rs) |
| Tool schemas exposed to the model | `ToolCatalog::native_tool_definitions_for_session(chat.tool_mode, warmed)` in [`tool_catalog.rs`](./crates/core/src/agent/tool_catalog.rs) |
| LLM message packing, trimming, token estimate | [`crates/core/src/agent/context.rs`](./crates/core/src/agent/context.rs) |
| Full tool args + result/error in context | `format_tool_context_message()` in `context.rs` |
| Harness nudges (missing args, duplicate tool, invalid name) | `tool_catalog.rs` + `push_harness_nudge` in `chat_loop.rs` — **chronological order**, not moved to tail |
| Mutating tool gate | `is_mutating_tool` → approval queue in `chat_loop.rs` / [`crates/core/src/engine/approvals.rs`](./crates/core/src/engine/approvals.rs) |
| Session persistence | [`crates/core/src/store/json.rs`](./crates/core/src/store/json.rs), [`crates/core/src/store/sqlite.rs`](./crates/core/src/store/sqlite.rs) (`data/chat/` when using JSON backend) |

Chat system prompt: [`prompts/chat.md`](./prompts/chat.md) — **embedded at build time** (`include_str!`); default `chat.prompt` does not read from cwd. Custom `chat.prompt` paths still load from disk for overrides.

Legacy JSON `action: reply | tool | approval` has been removed; chat uses native `tools` / `tool_calls` only.

---

## MCP integration

| Layer | Config / path | Role |
|-------|---------------|------|
| **GitHub** | `github:` in `coworker.yaml` | [`crates/core/src/github/harness.rs`](./crates/core/src/github/harness.rs) — in-process `gh`; meta-tools (`tool_list`, `tool_search`, `tool_describe`, `tool_call`) index GitHub + local harness only. |
| **Third-party** | `mcp.servers[]` | [`crates/core/src/mcp/`](./crates/core/src/mcp/) — `McpPool`; `transport: stdio` (subprocess) or `http` (Streamable HTTP POST + JSON/SSE). |

- Chat routes federated readonly MCP tools through `execute_readonly_tool` in [`chat_loop.rs`](./crates/core/src/agent/chat_loop.rs); mutating MCP tools are split out alongside GitHub harness mutators in the tool-call loop and queued through `queue_mutating_approval` (same approval queue as `ci_rerun_workflow`, etc.) unless `chat.auto_approve_mutations` or per-server `approval.mutating: auto` applies.
- Lazy mode: when `mcp.servers[]` is non-empty, `tool_list` / `tool_search` / `tool_describe` federate GitHub harness + MCP registry ([`crates/core/src/mcp/lazy_adapter.rs`](./crates/core/src/mcp/lazy_adapter.rs)).
- TUI Config tab shows per-server `mcp[id]` status from `AppState.mcp_servers`.

For new **GitHub** tools, extend **GithubHarness** / unistar-mcp catalog — do not duplicate `gh` calls in coworker. For **Slack/filesystem/etc.**, add an MCP server entry under `mcp.servers[]`.

---

## Store, scheduler, TUI

| Area | Path |
|------|------|
| JSON / SQLite store | `crates/core/src/store/` |
| Cron + workflow dispatch | `crates/core/src/engine/scheduler.rs`, `crates/core/src/engine/workflows.rs` |
| TUI (tabs, chat, context panel, approvals) | `crates/tui/src/` |
| CLI entry | `crates/unistar-coworker/src/main.rs`, `crates/cli/src/` |

Default store backend is JSON under `./data` (gitignored). SQLite backend and `store migrate` are built in.

---

## Configuration

- Example: [`coworker.example.yaml`](./coworker.example.yaml).
- Loaded from cwd or `~/.config/unistar-coworker/coworker.yaml` (see [`crates/core/src/config.rs`](./crates/core/src/config.rs)).
- Key knobs: `repos`, `llm.context_limit` (64K), `chat.max_turns`, `chat.max_tool_calls`, `chat.tool_mode`, `policy.auto_rerun_flaky`, `github:`, `mcp.servers[]`.

---

## Common commands

```sh
# One-time: Conventional Commits hook (Husky + commitlint at repo root)
npm install

# Fast dev loop — no frontend embed; Web UI served from web-ui/dist/ at runtime
cargo check
cargo check -p coworker-tui    # TUI-only when editing crates/tui/
cargo run -p unistar-coworker -- serve   # after: cd web-ui && npm run build:fast (once)

# Release / deploy — embed web-ui/dist into the binary
cargo build --release --features embed-web-ui

cargo fmt --check              # CI enforces formatting
cargo clippy --workspace --features embed-web-ui -- -D warnings
cargo test --workspace
cd web-ui && npm run build:fast && npx tsc --noEmit && npx vitest run
cargo run -p unistar-coworker --release --features embed-web-ui            # TUI + scheduler
cargo run -p unistar-coworker --release --features embed-web-ui -- chat --once "Summarize open PRs in acme/widget"
cargo run -p unistar-coworker --release --features embed-web-ui -- run-once --workflow daily-work
```

### Fast compile (dev)

The repo is a **Cargo workspace** (`crates/core`, `crates/tui`, `crates/web`, `crates/cli`, `crates/unistar-coworker`). Editing one surface crate avoids recompiling unrelated layers when their rlibs are still clean — use `cargo check -p coworker-tui` (etc.) for the tightest loop.

Default `cargo build` / `cargo check` **omit** `embed-web-ui`. The React UI is read from `web-ui/dist/` at runtime ([`crates/web/src/ui.rs`](./crates/web/src/ui.rs)), so changing only Rust code does not re-embed JS bundles. Use Vite HMR (`cd web-ui && npm run dev`) alongside `cargo run -p unistar-coworker -- serve` for frontend work.

Release builds, [`scripts/package.sh`](./scripts/package.sh), and CI use `--features embed-web-ui` for a single-binary deploy. Optional speedups: `.cargo/config.toml` sets `debug=1` + incremental; uncomment `sccache` / `mold` there if installed.

List skills/workflows: `cargo run --release --features embed-web-ui -- skills list` / `workflows list`.

---

## CI (required after `git push`)

**Every push to `main` / `master` triggers GitHub Actions.** Your changes must leave CI green — do not push and walk away if checks are likely to fail.

Before pushing (or immediately after, if you already pushed), run the same bar locally:

```sh
npm install   # if hooks not installed yet
./scripts/check-versions.sh
cd web-ui && npm install && npx tsc --noEmit && npm test && npm run build:fast
cargo fmt --check
cargo clippy --workspace --features embed-web-ui -- -D warnings
cargo test --workspace --features embed-web-ui
echo "feat(scope): subject" | npx commitlint   # optional sanity check
```

### CI jobs (`.github/workflows/ci.yml`)

| Job | What it runs |
|-----|----------------|
| **`rust`** | `check-versions.sh` → Web UI `tsc --noEmit` + `vitest` + `build:fast` → `cargo fmt` / `clippy` / `test` (`embed-web-ui`) |
| **`rust-no-default-features`** | Web UI build → `cargo check` / `test` with `--no-default-features` |
| **`web-e2e`** | Rust build + Playwright smoke tests (**blocking**) |
| **`docker-smoke`** | `docker build -t unistar-coworker:ci .` |
| **`secret-scan`** | gitleaks (`.gitleaks.toml`) |
| **`cargo-deny`** | `cargo deny check advisories` (blocking) |
| **`commitlint`** | Conventional Commits on PR / `main` pushes ([docs/COMMITS.md](./docs/COMMITS.md)) |

If you touch `Cargo.toml` features, `build.rs`, or optional deps, verify **`rust-no-default-features`** too.

When Web UI or `web-ui/dist` embedding changes, ensure `npm run build:fast` succeeds so Rust `build.rs` does not warn about a missing `web-ui/dist`.

Bump `[workspace.package].version` in `Cargo.toml` together with the **Crate version** lines in `README.md` and `README_CN.md` — `check-versions.sh` enforces sync.

If CI fails after your push, **fix and push again** until all jobs pass — do not leave broken `main`.


## Conventions for code changes

- **Commits** — [Conventional Commits 1.0.0](https://www.conventionalcommits.org/en/v1.0.0/); full spec in [docs/COMMITS.md](./docs/COMMITS.md). Format: `<type>[scope]: <imperative subject>` (e.g. `fix(cli): redact doctor bundle yaml`). Types: `feat`, `fix`, `docs`, `ci`, `chore`, `deps`, … Scopes: `core`, `cli`, `web`, `tui`, `web-ui`, `ci`, `docker`, `docs`, `skills`, `deps`, … — omit when cross-cutting. Breaking: `feat!:` or `BREAKING CHANGE:` footer. **Enforced** by Husky `commit-msg` (after root `npm install`) and CI job **`commitlint`** (`commitlint.config.mjs`). Do not use `--no-verify` unless the user explicitly asks; never put secrets in messages.
- **Minimal diff** — match existing style in the file; reuse `tool_catalog`, `context`, `parse` helpers instead of new one-off logic.
- **Rust 2021**, `tokio` async, `thiserror` / `anyhow` for errors.
- **Tests** — unit tests live next to modules (`mod tests`); use `acme/widget` and synthetic JSON; run full `cargo test` before finishing.
- **CI must pass** — see [CI (required after `git push`)](#ci-required-after-git-push); run fmt/clippy/test (+ Web UI build when relevant) before pushing; fix failures until green.
- **Comments** — only for non-obvious harness invariants; prefer clear names.
- **No new secrets** in repo; no real session dumps under `data/` in commits.
- **Mutating behavior** — must stay behind approval unless config explicitly opts out.
- **Context budget** — 64K-oriented; history uses ~40% of input; when over budget, older turns batch into one `[earlier context summary]` via LLM (`trim_llm_messages_with_llm`), then incremental trim if needed; harness nudges are never folded into summaries.

When adding a chat tool, update: `TOOLS.md` (if documented), `tool_catalog.rs` `TOOLS` table, and tests.

---

## Related repos

- [unistar-mcp](../unistar-mcp) — Go MCP server (`gh`/`git`); see its `AGENTS.md` for tool design principles.
- MCP PR/CI triage skill: `unistar-mcp/.cursor/skills/pr-ci-triage/SKILL.md` (coworker loads `skills/ci-triage/SKILL.md` for workflows/chat).
