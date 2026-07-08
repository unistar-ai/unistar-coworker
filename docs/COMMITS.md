# Commit message conventions

This repository follows [Conventional Commits 1.0.0](https://www.conventionalcommits.org/en/v1.0.0/).

Commit messages are **human-readable history** and input for **CHANGELOG** / **SemVer** decisions at release time. Write them for teammates and future you—not only for machines.

## Format

```
<type>[optional scope][optional !]: <description>

[optional body]

[optional footer(s)]
```

| Part | Rules |
|------|--------|
| **type** | Required noun: `feat`, `fix`, `docs`, … (see table below) |
| **scope** | Optional; lowercase; see [Scopes](#scopes) |
| **!** | Optional; marks a **breaking change** when appended before `:` |
| **description** | Required; imperative, lowercase start, no trailing period; ~72 chars |
| **body** | Optional; wrap at ~72 chars; explain *why*, not only *what* |
| **footer** | Optional; `BREAKING CHANGE:`, `Refs:`, `Reviewed-by:`, etc. |

## Types

| Type | When to use | SemVer (typical) |
|------|-------------|------------------|
| `feat` | New user-facing capability | MINOR |
| `fix` | Bug fix | PATCH |
| `docs` | Documentation only | — |
| `style` | Formatting, whitespace; no logic change | — |
| `refactor` | Restructure without behavior change | — |
| `perf` | Performance improvement | PATCH |
| `test` | Add or fix tests | — |
| `build` | Build system, `build.rs`, packaging scripts | — |
| `ci` | GitHub Actions, CI config | — |
| `chore` | Maintenance (deps, tooling) with no src behavior change | — |
| `revert` | Revert a prior commit | depends |

Other types from the [spec](https://www.conventionalcommits.org/en/v1.0.0/) are allowed when they fit better than forcing `feat`/`fix`.

## Breaking changes

A breaking change **must** be visible in the subject or footer:

1. **Subject:** `feat(api)!: remove legacy config key` — `!` before `:`
2. **Footer:**
   ```
   BREAKING CHANGE: environment variables now override coworker.yaml llm.base_url
   ```

Breaking commits correlate with **MAJOR** SemVer bumps. Record upgrade notes in [CHANGELOG.md](../CHANGELOG.md) before tagging.

## Scopes

Use a scope when the change is clearly localized. Omit scope for repo-wide or multi-area changes.

| Scope | Area |
|-------|------|
| `core` | `crates/core` — config, harness, workflows, LLM |
| `cli` | `crates/cli` — commands, doctor, init |
| `web` | `crates/web` — Axum server, embed |
| `tui` | `crates/tui` — ratatui UI |
| `web-ui` | `web-ui/` — React frontend |
| `ci` | `.github/workflows`, gitleaks, cargo-deny |
| `docker` | `Dockerfile`, `docs/docker.md` |
| `release` | `release.yml`, `scripts/package.sh`, dist |
| `packaging` | `packaging/`, workdir template |
| `docs` | README, `docs/`, policy files |
| `skills` | `skills/`, `prompts/` |
| `deps` | Dependency-only bumps (often Dependabot) |

## Examples

### Feature

```
feat(cli): add doctor --bundle for support zip export
```

### Bug fix with body

```
fix(web-ui): correct task-list detection under React 19 types

isValidElement props are stricter; use input element check instead of
generic props.type access.
```

### Breaking change

```
feat(core)!: require config_version in coworker.yaml

BREAKING CHANGE: existing configs without config_version are migrated on
load; back up coworker.yaml before upgrading past v3.0.0.
```

### CI / docs / chore

```
ci: upgrade GitHub Actions workflows to Node.js 24
```

```
docs: add commit message conventions
```

```
chore(deps): bump anyhow to 1.0.103 for RUSTSEC-2026-0190
```

### Revert

```
revert: let us never again speak of the noodle incident

Refs: 676104e
```

## Pull requests

- **One logical change per commit** when practical; split large PRs.
- **Squash merge:** edit the squash title/body to match this spec (GitHub’s default merge message is often too vague).
- **Dependabot:** after rebase/fix, squash title should stay meaningful, e.g. `chore(deps): bump tokio-tungstenite to 0.29.0`.
- **Never** put secrets, real tokens, or production `owner/repo` names in subjects or bodies — see [AGENTS.md](../AGENTS.md).

## Release linkage

| Commit pattern | Release bump |
|----------------|--------------|
| `fix:` | PATCH |
| `feat:` | MINOR |
| `BREAKING CHANGE` or `!` in subject | MAJOR |

Maintainers fold user-visible `feat` / `fix` / breaking items into [CHANGELOG.md](../CHANGELOG.md) under `[Unreleased]` before tagging. See [docs/releasing.md](./releasing.md).

## Enforcement (shell)

Commit messages are validated automatically — **no Node.js required**:

| Layer | What |
|-------|------|
| **Local** | `scripts/hooks/commit-msg` → [scripts/validate-commit-msg.sh](../scripts/validate-commit-msg.sh) |
| **CI** | Job `commit-messages` → [scripts/validate-commit-range.sh](../scripts/validate-commit-range.sh) |

Setup:

```bash
./scripts/setup-git-hooks.sh
```

Merge/revert commits (`Merge …`, `Revert …`) are skipped. To bypass the local hook in an emergency: `git commit --no-verify` (CI still checks on push/PR).

Manual check:

```bash
./scripts/validate-commit-msg.sh --subject "feat(cli): example"
./scripts/validate-commit-msg.sh --self-test
```

## References

- [Conventional Commits 1.0.0](https://www.conventionalcommits.org/en/v1.0.0/)
- [CONTRIBUTING.md](../CONTRIBUTING.md) — workflow and PR checks
- [Semantic Versioning 2.0.0](https://semver.org/)
