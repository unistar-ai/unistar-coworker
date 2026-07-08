# Release process (maintainers)

Internal guide for shipping unistar-coworker versions. User-facing upgrade steps: [upgrading.md](./upgrading.md).

## Version source of truth

- **Authoritative:** `[workspace.package].version` in root [Cargo.toml](../Cargo.toml)
- **CLI:** `unistar-coworker --version` (`CARGO_PKG_VERSION`)
- **Docs:** README / README_CN crate version line must match (CI `check-versions.sh` when added)

Bump only on `main` (or a release branch) immediately before tagging.

## Pre-release checklist

- [ ] [CHANGELOG.md](../CHANGELOG.md) — move items from `[Unreleased]` to `## [X.Y.Z] - YYYY-MM-DD`
- [ ] README / README_CN version line updated
- [ ] `cargo fmt --check`, `cargo clippy --workspace --features embed-web-ui -- -D warnings`, `cargo test --workspace`
- [ ] `cd web-ui && npx tsc --noEmit && npm test`
- [ ] `cargo check -p unistar-coworker` (refreshes `Cargo.lock` if version changed)

## Tag and publish

```bash
# On main, after version bump commit:
git tag v2.1.0
git push origin v2.1.0
```

Pushing `v*` triggers [.github/workflows/release.yml](../.github/workflows/release.yml):

| Matrix | Artifact |
|--------|----------|
| `ubuntu-latest` | `unistar-coworker-v2.1.0-x86_64-unknown-linux-gnu.tar.gz` + `.sha256` + `.sbom.json` |
| `macos-latest` | `unistar-coworker-v2.1.0-aarch64-apple-darwin.tar.gz` + `.sha256` |

Pushing `v*` also triggers [.github/workflows/docker.yml](../.github/workflows/docker.yml) (GHCR image).

`scripts/package.sh` runs per matrix row with `PACKAGE_VERSION` and `PACKAGE_TRIPLE` from the workflow.

## GitHub Release notes

Use this template in addition to auto-generated commits:

```markdown
## Highlights
- …

## Upgrade notes
- …

## Assets
- Linux x86_64 tar.gz + SBOM (CycloneDX JSON)
- macOS arm64 tar.gz
- Docker: `ghcr.io/unistar-ai/unistar-coworker:X.Y.Z`

Full changelog: [CHANGELOG.md#x-y-z](https://github.com/unistar-ai/unistar-coworker/blob/main/CHANGELOG.md#x-y-z)
```

## Post-release

- [ ] Verify assets on [Releases](https://github.com/unistar-ai/unistar-coworker/releases)
- [ ] Smoke: download tar.gz, `doctor`, `serve` on one Linux and one macOS arm64 machine
- [ ] Open `[Unreleased]` section in CHANGELOG for the next cycle
- [ ] (M2+) Confirm Docker tag on GHCR
- [ ] (M4+) Dependabot / advisory scan follow-up if needed

## Semantic versioning

| Bump | When |
|------|------|
| **MAJOR** | Breaking **Stable** RPC, exit codes, or config semantics |
| **MINOR** | Features, docs milestones (M1–M4), new Stable RPC ops |
| **PATCH** | Bug fixes only |

Align milestone releases with the product readiness plan when applicable (e.g. M1 → v2.1.0).

## Security releases

For embargoed fixes: use GitHub Security Advisories, patch branch, tag `vX.Y.Z`, and note in CHANGELOG under `### Security`.
