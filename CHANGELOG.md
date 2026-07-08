# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [2.3.0] - 2026-07-08

### Added

- `coworker.minimal.yaml` — workspace-only config template (no GitHub).
- `skills/general-agent-tone` — default always-on reply style (tool-grounded, non-secretary).
- `docs/local-models.md` — 25B+ reference models (gemma 26B A4B, qwen3.6-27B), `tool_mode`, and chat knobs.
- `doctor` checks: `llm-model` / `llm-context` hints for **25B+** reference tier; GitHub auth is **warn** when `repos:` is empty.

### Changed

- Product positioning: **local-first general agent** for local LLMs; GitHub/MCP/workflows are optional capability packs.
- Default chat prompt (`prompts/chat.md`): general workspace agent tone; GitHub only when skills/user ask.
- `github-ops-tone`: optional domain skill (`always: false`); load for GitHub/CI ops.
- Skill directory fallback (`prompt.rs`): `general-agent-tone` + `code-edit` when `skills/` is missing.
- `QUICKSTART*` / README: workspace + Ollama first; GitHub optional second section.
- `coworker.example.yaml` / `coworker.minimal.yaml`: default `tool_mode: auto`; model `gemma4:26b-a4b-it-qat`.
- `skills/code-edit`: full explore → patch → verify workflow.
- README Features table: Chat / LLM / workspace before GitHub workflows.

[2.3.0]: https://github.com/unistar-ai/unistar-coworker/compare/v2.1.0...v2.3.0

## [2.1.0] - 2026-07-08

### Added

- Policy and community docs: `SECURITY.md`, `PRIVACY.md`, `SUPPORT.md`, `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`, `CHANGELOG.md`.
- User guides: `docs/STABILITY.md`, `docs/upgrading.md`, `docs/troubleshooting.md`, `docs/releasing.md`, `docs/docker.md`.
- `QUICKSTART.md` / `QUICKSTART_CN.md`; included in `package.sh` output.
- Docker: multi-stage `Dockerfile`, `.github/workflows/docker.yml` (GHCR on tag).
- `unistar-coworker init --interactive` (TTY prompts, Ollama probe, repo validation).
- `unistar-coworker upgrade-check [--json]` (GitHub Releases API, no telemetry).
- `unistar-coworker doctor --bundle <zip>` (redacted diagnostic export).
- LLM `api_key` `${ENV_VAR}` expansion; `.env.example`.
- `scripts/check-versions.sh`; GitHub Issue templates; `dependabot.yml`; `CODEOWNERS`.
- CI: web-ui `tsc` + vitest; blocking Playwright e2e; `docker-smoke`; `cargo-deny` (blocking); gitleaks secret scan.
- `config_version` field + `migrate_config_value()` framework in config loader.
- `serve` background `upgrade-check`; Web Config shows version + update link.
- RPC stable error shape tests; release SBOM (Linux, CycloneDX JSON).

### Changed

- Workspace version 2.1.0; README policy links, supported platforms table, Docker quick start.
- `doctor` checks: unresolved env placeholders, plaintext secrets, `0.0.0.0` bind without `auth_token`, data dir writable, port 8787 warn.
- Missing `coworker.yaml` hints suggest `init --interactive`.
- `docs/RPC.md`: Stable op labels + link to `STABILITY.md`.

### API

- RPC ops `chat`, `get_state`, `cancel`, `switch_profile` documented as **Stable** per `docs/STABILITY.md`.

[2.1.0]: https://github.com/unistar-ai/unistar-coworker/compare/v2.0.1...v2.1.0

## [2.0.1] - 2026-07

### Changed

- Unified release packaging in `scripts/package.sh` (build + workdir refresh).
- Renamed packaging script and limited it to deploy builds.

[2.0.1]: https://github.com/unistar-ai/unistar-coworker/compare/v2.0.0...v2.0.1

## [2.0.0] - 2026-07

### Added

- GitHub Releases workflow for tagged builds (Linux x86_64, macOS arm64 tarballs).
- Cargo workspace split (`core`, `tui`, `web`, `cli`, `unistar-coworker`) for faster incremental builds.
- Pi-style scripting: JSONL `rpc` mode, session branches, `export session`, stable exit codes.
- Runtime LLM profile switching (`llm_profiles`, Web Config / RPC `switch_profile`).
- Packaging workdir template and in-repo `scripts/package.sh`.

### Changed

- Major version alignment for first formal release line (2.x).

[2.0.0]: https://github.com/unistar-ai/unistar-coworker/releases/tag/v2.0.0
