# Skill authoring template

Copy this file to `skills/<name>/SKILL.md` and replace placeholders. Skills are **technique packs** — routing hints, tool chains, and tone — not executable code.

Tool names must match [`TOOLS.md`](./TOOLS.md) and [`tool_catalog.rs`](../../crates/core/src/agent/tool_catalog.rs).

---

## Frontmatter (required)

```yaml
---
name: my-skill
description: "One line: when to load this skill. Use when <user intent>."
# always: false          # true = inject on every chat turn (use sparingly; see general-agent-tone)
# argument-hint: "PR #, file path, or error snippet"
# intent_keywords: [ci, fail, build]       # skill router bonus
# intent_bonus_keywords: [pr, "#"]
# tools:                  # warm these MCP/harness tools when skill loads (chat)
#   - pr_get_overview
#   - ci_get_failure_digest
---
```

| Field | Purpose |
|-------|---------|
| `name` | Directory name and router id (kebab-case) |
| `description` | Shown in **Available skills**; primary routing signal |
| `always` | `true` only for global tone/behavior (default: omit or `false`) |
| `intent_keywords` | Extra match weight in skill router |
| `tools` | Pre-warm tool schemas in chat (`tool_mode: auto` / `lazy`) |

---

## Body structure (recommended)

```markdown
# My Skill Title

One sentence: what this skill optimizes for.

## Scope

Use for:
- …

Not for:
- … → point to `other-skill`

## Workflow

1. **Anchor** — gather `repo`, paths, or ids from user text.
2. **Read first** — cheapest tool that answers the question.
3. **Deep dive** — logs/diffs only when digest is insufficient.
4. **Summarize** — verdict + evidence + one next step.

## Output

- Bullet format expectations
- Tables for verdicts / classifications

## Guardrails

- Do not invent tool output
- Cap log reads; mention truncation
- Mutating tools → approval queue (chat only)
```

---

## Checklist before merge

- [ ] `description` is a complete “Use when …” line (not a title alone)
- [ ] Tool names exist in `TOOLS.md` / `tool_catalog.rs`
- [ ] **Scope** lists at least one “Not for” redirect
- [ ] No Rust harness logic in the skill body
- [ ] `cargo run --release -- skills list` shows the skill
- [ ] If GitHub-specific, add a row to [`github-ops-pack/README.md`](../github-ops-pack/README.md)

---

## Examples in this repo

| Skill | Pattern |
|-------|---------|
| [`general-agent-tone`](../general-agent-tone/SKILL.md) | `always: true` — global reply style |
| [`code-edit`](../code-edit/SKILL.md) | Workspace coding workflow |
| [`ci-triage`](../ci-triage/SKILL.md) | MCP tool chain + verdict table |
| [`github-ops-tone`](../github-ops-tone/SKILL.md) | Optional domain tone (`always: false`) |

See also [`AGENTS.md`](../../AGENTS.md) § Skill / Prompt / Harness.
