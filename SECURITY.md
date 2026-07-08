# Security Policy

## Supported versions

Security fixes are provided for the **latest minor release** within the current **major** version line.

| Version | Supported |
|---------|-----------|
| 2.1.x (latest 2.x) | Yes |
| 2.0.x | No — upgrade to the latest 2.x release |
| 1.x and earlier | No |

We do not commit to fixing vulnerabilities in end-of-life (EOL) releases. When in doubt, run `unistar-coworker --version` and upgrade to the newest 2.x release from [GitHub Releases](https://github.com/unistar-ai/unistar-coworker/releases).

## Reporting a vulnerability

**Please do not open a public GitHub Issue for security vulnerabilities.**

Report privately via one of:

1. **[GitHub Security Advisories](https://github.com/unistar-ai/unistar-coworker/security/advisories)** — preferred; use *Report a vulnerability* on the repository Security tab.
2. **Private vulnerability reporting** — if enabled in repository Settings → Security.

Include:

- Affected version(s) and install method (tar.gz, Docker, source build)
- Steps to reproduce and impact assessment
- Any proof-of-concept (redact real tokens and production repo names)

We aim to acknowledge reports within a reasonable timeframe. There is no commercial SLA; this is a community-maintained local tool.

## Local deployment model

unistar-coworker is a **single-user, localhost-first** application. It is not designed for untrusted multi-user or public internet exposure.

- Default Web UI bind: `127.0.0.1:8787` — keep this for normal use.
- If you bind beyond localhost (e.g. `0.0.0.0`), you **must** set `web.auth_token` in `coworker.yaml`.
- Docker: map ports to localhost only, e.g. `-p 127.0.0.1:8787:8787`.
- Do not expose the Web UI through a public reverse proxy without strong authentication.

See also [PRIVACY.md](./PRIVACY.md) and the Web security section in [README.md](./README.md).

## Secrets and credential leaks

**Never commit** `coworker.yaml`, `.env`, or `data/` — they may contain API keys and session data.

- Store secrets in environment variables (`${VAR}` placeholders in config) or your shell/Docker `-e` flags.
- If a key is leaked (committed to git, pasted in an issue, or exposed via misconfigured bind):
  1. **Rotate the credential immediately** at the provider (GitHub, LLM vendor, MCP server, etc.).
  2. Remove the secret from git history if it was committed.
  3. Revoke and re-issue tokens; we cannot rotate credentials on your behalf.

The project maintainers are not responsible for credentials you place in config files or logs.

## Dependency security

Release builds use locked dependencies (`Cargo.lock`, `web-ui/package-lock.json`). Security advisories for dependencies are addressed through normal release and Dependabot workflows when applicable.
