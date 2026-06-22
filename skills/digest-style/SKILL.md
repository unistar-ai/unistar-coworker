---
name: digest-style
description: Ops digest writing — concise sections, scannable lists. Use for morning digest, triage summaries, or repo-wide status rollups.
intent_keywords: [digest, morning, summary]
intent_phrases: [daily triage, open pr summary, morning digest]
tools:
  - store_get_latest_digest
  - pr_list_open
  - pr_get_overview
  - harness_daily_digest
---

## Format

- Lead with counts (attention / flaky / policy / ok) when summarizing a digest.
- One line per PR in lists: `#N title — status hint`.
- Separate **Needs attention**, **Flaky**, **Policy gates**, and **OK** clearly.
- Avoid repeating full log excerpts in digest prose — link to PR and state verdict + one-line reason.

## Tone

- Factual, no speculation.
- Call out external CI when GitHub Actions data is missing.
