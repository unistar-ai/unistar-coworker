# Support

unistar-coworker is a **free, open-source, local personal tool**. It is **not** a hosted service or team product.

## How to get help

**GitHub Issues only** — [open an issue](https://github.com/unistar-ai/unistar-coworker/issues/new/choose) and choose a template:

| Template | Use for |
|----------|---------|
| **Bug report** | Crashes, incorrect behavior, regressions |
| **Feature request** | New capabilities (respecting local/single-user scope) |
| **Question** | Usage, configuration, upgrade questions |

Please include:

- `unistar-coworker --version`
- Platform (Linux x86_64 / macOS arm64 / other)
- Install method (tar.gz release / Docker / source `cargo build`)
- Output of `unistar-coworker doctor --json` (**redact** API keys and tokens)

## What we do not offer

- **No SLA** — no guaranteed response time or uptime
- **No commercial support** — no paid tiers or dedicated support channels
- **No multi-user / enterprise deployment** — designed for one operator on one machine; team RBAC and centralized ops are out of scope (fork or integrate locally if needed)

## Self-service resources

| Resource | Topic |
|----------|-------|
| [README.md](./README.md) / [README_CN.md](./README_CN.md) | Overview, quick start, configuration |
| [docs/troubleshooting.md](./docs/troubleshooting.md) | Common problems and fixes |
| [docs/upgrading.md](./docs/upgrading.md) | Version upgrades |
| [docs/RPC.md](./docs/RPC.md) | JSONL scripting API |
| [CONTRIBUTING.md](./CONTRIBUTING.md) | Contributing code or docs |
| [QUICKSTART.md](./QUICKSTART.md) / [QUICKSTART_CN.md](./QUICKSTART_CN.md) | Step-by-step install (tar.gz + Docker) |

## Supported platforms

Official release binaries:

- **Linux x86_64** — tar.gz and Docker (GHCR)
- **macOS arm64 (Apple Silicon)** — tar.gz

Other platforms (Intel Mac, Linux arm64, Windows): build from source — **community best-effort**, not officially supported.

## Security issues

Do **not** use public issues for vulnerabilities. See [SECURITY.md](./SECURITY.md).
