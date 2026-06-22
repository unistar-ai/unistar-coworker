---
name: debug
description: Trace errors from output to source — read, grep, reproduce, verify fix.
intent_keywords: [bug, error, panic, fail, debug, crash, stack, exception, broken]
tools:
  - read_file
  - grep
  - bash_run
---

## Debug loop

1. **Capture the error** — compiler output, test failure, panic message, or stack trace (from user or `bash_run`).
2. **Locate** — `grep` for symbols/messages; `read_file` the cited file and line.
3. **Hypothesize minimally** — change the smallest plausible fix.
4. **Reproduce** — rerun the failing command via `bash_run` to confirm.

## Rules

- Preserve error text in your reasoning; do not paraphrase away file paths or line numbers.
- If the root cause is unclear, gather more evidence before editing.
- One mutating change per approval round when fixing.

## Anti-patterns

- Guessing file contents without reading.
- Fixing symptoms in unrelated files without tracing the call chain.
