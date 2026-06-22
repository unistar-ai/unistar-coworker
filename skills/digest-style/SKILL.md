---
name: digest-style
description: "Write ops digests and triage rollups — scannable sections, counts first. Use for morning digest, repo-wide status summaries, or formatting harness digest output for humans."
argument-hint: "Audience and time window for the digest"
intent_keywords: [digest, morning, summary]
intent_phrases: [daily triage, open pr summary, morning digest]
tools:
  - store_get_latest_digest
  - pr_list_open
  - pr_get_overview
  - harness_daily_digest
---

# Digest Style

Ops secretary tone: factual, scannable, no log dumps in prose.

## Scope

Use for:
- Morning digests, triage summaries, PR rollups
- Repackaging tool/store output for humans

Do not:
- Speculate beyond tool data
- Paste full CI logs — link PR and state verdict + one-line reason

## Workflow

1. **Gather** — `store_get_latest_digest` and/or `harness_daily_digest`, `pr_list_open` as needed.
2. **Bucket** — Needs attention / Flaky / Policy gates / OK.
3. **Lead with counts** when summarizing a digest.
4. **One line per PR** — `#N title — status hint`.
5. **External CI** — call out when Actions data is missing.

## Output template

### Headline counts
Attention · Flaky · Policy · OK

### Needs attention
- `#N` — one-line reason

### Flaky / Policy / OK
(sections as needed, same one-line format)

### Notes
External CI, missing data, or store staleness
