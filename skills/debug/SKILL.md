---
name: debug
description: "Trace errors from output to source and verify fixes. Use when tests fail, builds break, panics occur, or the user pastes a stack trace."
argument-hint: "Error message, command, or stack trace"
intent_keywords: [bug, error, panic, fail, debug, crash, stack, exception, broken]
tools:
  - read_file
  - grep
  - bash_run
---

# Debug

Follow evidence from symptom to root cause. Hypothesize minimally; confirm with reruns.

## Scope

Use for:
- Compiler/test/runtime failures in the local workspace
- Tracing symbols and messages to source lines

Do not:
- Guess file contents without reading
- Patch unrelated files without following the call chain

## Workflow

1. **Capture** — preserve exact error text, paths, and line numbers from user or `bash_run`.
2. **Locate** — `grep` symbols/messages; `read_file` cited locations.
3. **Hypothesize** — smallest change that explains the failure.
4. **Fix** — one mutating change per approval round (load `code-edit` if needed).
5. **Reproduce** — rerun the **same** failing command via `bash_run`.

## Output template

### Symptom
Exact error excerpt

### Root cause
One paragraph with file:line evidence

### Fix / next step
What changed or what to gather next
