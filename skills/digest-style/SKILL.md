---
name: digest-style
description: Ops digest writing — concise sections, scannable lists.
---

## Format

- Lead with counts (attention / flaky / policy / ok) when summarizing a digest.
- One line per PR in lists: `#N title — status hint`.
- Separate **Needs attention**, **Flaky**, **Policy gates**, and **OK** clearly.
- Avoid repeating full log excerpts in digest prose — link to PR and state verdict + one-line reason.

## Tone

- Factual, no speculation.
- Call out external CI when GitHub Actions data is missing.
