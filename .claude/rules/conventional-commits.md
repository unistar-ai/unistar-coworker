# Conventional Commits

Follow [docs/COMMITS.md](../../docs/COMMITS.md) and [Conventional Commits 1.0.0](https://www.conventionalcommits.org/en/v1.0.0/).

Format: `<type>[optional scope][optional !]: <description>`

Types: `feat`, `fix`, `docs`, `style`, `refactor`, `perf`, `test`, `build`, `ci`, `chore`, `revert`, `deps`.

Scopes (this repo): `core`, `cli`, `web`, `tui`, `web-ui`, `ci`, `docker`, `release`, `packaging`, `docs`, `skills`, `deps` — omit when cross-cutting.

Enforcement: `./scripts/setup-git-hooks.sh` (local shell hook); CI job `commit-messages`.

Do not use `git commit --no-verify` unless the user explicitly asks. Never put secrets in commit messages.
