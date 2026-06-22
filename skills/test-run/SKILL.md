---
name: test-run
description: "Run and interpret tests, builds, and linters. Use when the user wants cargo/npm/pytest results, compile checks, or verification after a change."
argument-hint: "Test command or module to run"
intent_keywords: [test, cargo, npm, pytest, build, compile, run, check, lint]
tools:
  - bash_run
  - read_file
---

# Test Run

Run the narrowest command that answers the question. Report exit codes and real output — never invent pass/fail.

## Scope

Use for:
- Targeted test filters and build/lint commands
- Interpreting failure output before editing

| Stack | Typical commands |
|-------|------------------|
| Rust | `cargo test`, `cargo build`, `cargo clippy` |
| Node | `npm test`, `npm run build`, `pnpm test` |
| Python | `pytest`, `python -m pytest` |

## Workflow

1. **Choose scope** — filtered tests when the user scoped the issue (`cargo test mod_name`, `pytest path::test`).
2. **Run** — `bash_run` with the chosen command.
3. **Read failures** — focus on the **first** actionable error; note file:line.
4. **Deep dive** — `read_file` at cited location if a fix is next.
5. **Confirm** — after edits, rerun the **same** command.

## Output template

### Command
`...`

### Result
Pass/fail, exit code, relevant tail of output

### First error (if failed)
File:line and one-line interpretation
