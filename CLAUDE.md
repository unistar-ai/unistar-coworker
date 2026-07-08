# CLAUDE.md

Instructions for **Claude Code** in this repository.

## Start here

**Canonical guide (all AI tools):** [AGENTS.md](./AGENTS.md) — architecture, harness, sensitive data, CI, conventions.

Read AGENTS.md before making changes. This file only lists Claude-specific entry points so we do not duplicate that document.

## Project in one line

**Local-first general agent for local LLMs** (Rust + TUI + Web UI). GitHub/MCP/workflows are capability packs; mutating external tools need approval unless config opts in.

## Common tasks

| Task | Where |
|------|--------|
| Build / test Rust | `cargo fmt --check`, `cargo clippy --workspace --features embed-web-ui -- -D warnings`, `cargo test --workspace` |
| Web UI | `cd web-ui && npm install && npx tsc --noEmit && npm test && npm run build:fast` |
| Commit messages | [docs/COMMITS.md](./docs/COMMITS.md); run `./scripts/setup-git-hooks.sh` once |
| Contribute / PR | [CONTRIBUTING.md](./CONTRIBUTING.md) |
| Release (maintainers) | [docs/releasing.md](./docs/releasing.md) |

## Tooling layout

| Path | Tool | Role |
|------|------|------|
| [AGENTS.md](./AGENTS.md) | Cursor, Claude, others | **Single source of truth** for agents |
| [.cursor/rules/](./.cursor/rules/) | Cursor IDE | IDE rules (e.g. Conventional Commits) |
| [.claude/rules/](./.claude/rules/) | Claude Code | Same policies, Claude-native rules |
| [skills/](./skills/) | unistar-coworker runtime | Workflow/chat skills (not IDE config) |

## Hard rules

- Never commit secrets, real `owner/repo`, or `coworker.yaml` / `data/` contents — see AGENTS.md.
- Keep diffs minimal; match existing style.
- Leave CI green after push (`main` runs full GitHub Actions).
