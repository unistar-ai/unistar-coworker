# API stability policy

This document defines compatibility promises for integrators and script authors using unistar-coworker outside the Web UI.

**Scope:**

- JSONL **RPC** protocol ([RPC.md](./RPC.md))
- **HTTP JSON API** (`/api/*`, `/ws` snapshot shapes)
- **CLI exit codes** ([`exit_codes.rs`](../crates/cli/src/exit_codes.rs))

## Stability levels

| Level | Meaning | Examples |
|-------|---------|----------|
| **Stable** | Semantics preserved across **minor** and **patch** releases; removal or breaking semantic change only in a **major** release | RPC ops `chat`, `get_state`, `cancel`, `switch_profile`; exit codes `0`–`4` |
| **Unstable** | May change in a minor release; clients should tolerate unknown fields | `WebSnapshot` and related WebSocket payloads; new optional JSON fields on `/api/*` responses |
| **Internal** | No compatibility guarantee | Undocumented HTTP routes; Web UI–only fields; private struct fields not in docs |

## Versioning rules

- **MAJOR** (e.g. 3.0.0): may remove or change semantics of **Stable** RPC operations or documented exit codes.
- **MINOR** (e.g. 2.2.0): may add RPC ops, HTTP routes, and JSON fields; must not break **Stable** ops.
- **PATCH** (e.g. 2.1.1): bug fixes only; no intentional breaking changes to **Stable** surface.

## Client guidance

1. **Ignore unknown JSON fields** — forward-compatible parsers should not fail on extra keys.
2. **Pin major version** for automation if you cannot tolerate rare breaking RPC changes at major bumps.
3. **Prefer Stable RPC ops** for long-lived scripts; see annotations in [RPC.md](./RPC.md).
4. **Web UI coupling** — the React app may use **Internal** shapes; do not rely on browser network traffic as a public API.

## Changelog

API-visible changes are noted under `### API` in [CHANGELOG.md](../CHANGELOG.md).

## Questions

Open a [GitHub Issue](https://github.com/unistar-ai/unistar-coworker/issues/new/choose) or see [SUPPORT.md](../SUPPORT.md).
