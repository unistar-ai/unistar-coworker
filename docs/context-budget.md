# Context budget and compaction

How unistar-coworker fits chat history, tool results, and skills into the LLM context window — especially for **25B+** local models at **64K–128K**.

See also: [local-models.md](./local-models.md), [AGENTS.md](../AGENTS.md) (context section).

---

## Token budget

| Piece | Typical share | Notes |
|-------|---------------|--------|
| System prompt + skills | Fixed per session | `general-agent-tone` always-on; more skills via `skill_load` |
| Tool schemas | Variable | `tool_mode: auto` / `native` sends more definitions than `lazy` |
| Message history | ~40% of input budget | User/assistant/tool turns |
| Headroom | Reserved | Avoid filling to 100% — trimming starts before hard limit |

Configured via `llm.context_limit` (default **64000**). The harness estimates tokens and trims when over budget.

---

## Trimming order

When estimated tokens exceed the budget:

1. **LLM batch summary** (optional) — older turns rolled into `[earlier context summary]` via `trim_llm_messages_with_llm`.
2. **Incremental trim** — `trim_llm_messages` compresses oldest tool results, then drops/summarizes middle messages while keeping a **tail** of recent turns (default protect last ~8 messages).
3. **Harness nudges** are never folded into summaries — they stay addressable for the model.

Recent tool results (verification `bash_run`, `edit_file` outcomes) are prioritized in the protected tail during coding sessions.

---

## `chat.compaction`

Controls how tool results and history batches are summarized when space is tight.

```yaml
chat:
  compaction: code   # default for workspace coding
  # compaction: ops      # GitHub/CI sessions — keep CI_KIND, verdicts, PR refs
  # compaction: generic  # domain-neutral LLM summaries
  # compaction:
  #   strategy: code
  #   summary_model: fast   # optional lighter LLM profile for summaries only
```

| Strategy | Use when | Keeps |
|----------|----------|--------|
| **`code`** (default) | Workspace coding, tests, edits | Exit codes, errors, paths, edit targets |
| **`ops`** | PR/CI triage, digests, MCP ops | `CI_KIND`, verdicts, PR numbers, digest excerpts |
| **`generic`** | Mixed / unknown | Short neutral summaries |

`summary_model` lets compaction use a faster/cheaper profile (e.g. remote API) while the main chat stays on your local 25B+ model.

---

## Session limits

```yaml
chat:
  max_turns: 0           # 0 = unlimited LLM rounds (default)
  max_tool_calls: 0      # 0 = unlimited tool executions per session
  max_duration_secs: 900 # wall clock cap (default 15 min)
  llm_step_timeout_secs: 180   # per LLM step; raise for slow local 25B+ cold start
  reasoning_only_warn_secs: 30 # Web/TUI: warn when only reasoning tokens grow
```

For long multi-file tasks on 64K context, defaults are usually fine. Set `max_turns` only to bound runaway loops.

---

## Tuning for 25B+ local models

| Goal | Suggestion |
|------|------------|
| Long coding sessions | `context_limit: 64000` or `128000` if the model supports it |
| Preserve build/test output | `compaction: code` (default) |
| GitHub-heavy chat | `compaction: ops` + `skill_load github-ops-tone` |
| Faster compaction | `compaction: { strategy: code, summary_model: remote-fast }` |
| Tight VRAM | `tool_mode: lazy` — smaller tool schema in context |

---

## UI behavior

- **Web UI** — long tool output collapses automatically; bash-style output shows **head + tail** when collapsed (exit line visible).
- **TUI** — scroll transcript; context panel shows token estimates when available.

---

## Verify

```bash
./unistar-coworker doctor    # context_limit / model tier
# After a long chat session, open Web → Chat → Context panel for token breakdown
```
