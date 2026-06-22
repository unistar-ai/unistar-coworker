---
name: test-run
description: Run and interpret tests and builds — cargo, npm, pytest, etc.
intent_keywords: [test, cargo, npm, pytest, build, compile, run, check, lint]
tools:
  - bash_run
  - read_file
---

## When to run what

| Signal | Typical command |
|--------|-----------------|
| Rust project | `cargo test`, `cargo build`, `cargo clippy` |
| Node / frontend | `npm test`, `npm run build`, `pnpm test` |
| Python | `pytest`, `python -m pytest` |

## Reading failures

- Focus on the **first** actionable error (compile error beats cascade of test failures).
- Note file:line from compiler/test output; `read_file` that location before editing.
- After a fix, rerun the **same** command to confirm.

## Rules

- Prefer targeted test filters (`cargo test module_name`, `pytest path::test`) when the user scoped the issue.
- Report exit code and the relevant tail of output; do not invent pass/fail.

## Anti-patterns

- Running the full suite repeatedly when a single test would suffice.
- Declaring success without rerunning after an edit.
