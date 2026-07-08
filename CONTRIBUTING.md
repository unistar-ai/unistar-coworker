# Contributing to unistar-coworker

Thank you for your interest in contributing. This project is a **local, single-user GitHub ops secretary** — contributions should respect that scope (no multi-tenant SaaS features unless clearly scoped as optional).

## Before you start

1. Read [AGENTS.md](./AGENTS.md) for architecture, harness conventions, sensitive-data rules, and CI expectations.
2. Check [open issues](https://github.com/unistar-ai/unistar-coworker/issues) to avoid duplicate work.
3. For large changes, open an issue first to discuss approach.

## Development setup

```bash
git clone https://github.com/unistar-ai/unistar-coworker.git
cd unistar-coworker

# Rust
cargo check
cargo build

# Web UI (when touching frontend or embed-web-ui)
cd web-ui && npm install && npm run build:fast
```

## Workflow

1. **Fork** the repository on GitHub.
2. **Branch** from `main` — use a descriptive name (`fix/doctor-llm-warn`, `docs/upgrading`).
3. **Make changes** — keep diffs minimal; match existing style in each file.
4. **Test locally** (see below).
5. **Open a pull request** against `main` with a clear description and test notes.

## Required checks before PR

Run the same bar as CI:

```bash
cargo fmt --check
cargo clippy --workspace --features embed-web-ui -- -D warnings
cargo test --workspace

cd web-ui
npm ci   # or npm install
npx tsc --noEmit
npm test   # vitest
```

If you change `Cargo.toml` features, optional deps, or `build.rs`, also verify:

```bash
cargo test --workspace --no-default-features
```

When Web UI or embedding changes, ensure `npm run build:fast` succeeds so `build.rs` does not warn about a missing `web-ui/dist/`.

## Commit style

- Use clear, imperative subject lines (`fix doctor warn on 0.0.0.0 bind`, `docs: add upgrading guide`).
- One logical change per commit when practical.
- **Never** commit secrets, real `owner/repo` names from production, or contents of `coworker.yaml` / `data/`.

## Code conventions

Summarized from [AGENTS.md](./AGENTS.md):

- **Rust 2021**, `tokio` async, `thiserror` / `anyhow` for errors.
- Unit tests live next to modules; use `acme/widget` and synthetic JSON.
- Mutating GitHub/MCP tools stay behind approval unless config explicitly opts out.
- New chat tools: update `skills/_base/TOOLS.md`, `tool_catalog.rs`, and tests together.

## Documentation

- User-facing docs: `README.md`, `README_CN.md`, `docs/`.
- Policy docs: `SECURITY.md`, `PRIVACY.md`, `SUPPORT.md`.
- Update [CHANGELOG.md](./CHANGELOG.md) for user-visible changes in releases (maintainers may fold this at release time).

## Code of conduct

This project follows the [Contributor Covenant](./CODE_OF_CONDUCT.md). Be respectful and constructive in issues and PRs.

## Questions

Use a [GitHub Issue](https://github.com/unistar-ai/unistar-coworker/issues/new/choose) with the **Question** template, or see [SUPPORT.md](./SUPPORT.md).
