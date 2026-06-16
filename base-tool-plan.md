# Base Tools Plan

> SSOT for tools shared across **Chat**, **workflows**, and **skills**.  
> Status: **plan** — partial implementation (`store_get_latest_digest` in chat only).

## Goals

1. **One vocabulary** — same tool names and semantics everywhere (chat agent, daily-work, merge-health, etc.).
2. **Two layers** — MCP (GitHub via unistar-mcp) vs Harness (local Store virtual tools).
3. **Composable skills** — workflow skills inherit a `_base` tool catalog instead of duplicating tables.
4. **Small prompt surface** — base set stays ~10–15 read-only tools; lazy `tool_list` / `tool_describe` / `tool_call` remain escape hatches.

## Non-goals

- Mutating tools in the base set (rerun, comment, backport stay **approval-only**).
- Shell, git, or filesystem tools in coworker.
- 20+ tools inlined in every system prompt.
- Replacing workflow-specific tools (e.g. `pr_get_diff` for light-review stays workflow-scoped).

---

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│  Skill (chat / daily-work / merge-health / …)               │
│    skills/_base/TOOLS.md  ← merged at load time               │
└───────────────────────────┬─────────────────────────────────┘
                            │
┌───────────────────────────▼─────────────────────────────────┐
│  Config: tools.base.mcp + tools.base.harness                  │
│    merged into chat.preferred_tools (chat)                    │
│    referenced by workflows via skill                          │
└───────────────┬─────────────────────────┬─────────────────────┘
                │                         │
     ┌──────────▼──────────┐   ┌──────────▼──────────┐
     │  unistar-mcp        │   │  harness_tools.rs   │
     │  (Go registry)      │   │  (Rust virtual)     │
     └─────────────────────┘   └─────────────────────┘
```

### MCP base (GitHub)

Read-only, high-frequency tools already in unistar-mcp:

| Tool | Purpose |
|------|---------|
| `pr_get_overview` | Single PR snapshot (status, files, failing runs) |
| `pr_get_merge_blockers` | Structured merge blockers |
| `pr_list_waiting_review` | CI-green + review-required list |
| `pr_list_open` | Open PR list (compact lines) |
| `pr_get_status` | Lightweight status check |
| `pr_list_merged` | Recently merged PRs |
| `ci_get_run_summary` | Run summary before full logs |
| `ci_analyze_pr_failures` | PR-scoped failure analysis |
| `ci_get_failed_logs` | Failed job logs (capped) |
| `issue_list_open` | Open issues |
| `issue_get` | Single issue |
| `alert_list_open` | Dependabot / code scanning alerts |

**Optional Phase 4:** `repo_get_info` (default branch, labels, team hints) if chat keeps asking meta questions.

### Harness base (Store virtual)

Executed in coworker Rust — no GitHub API:

| Tool | Purpose | Status |
|------|---------|--------|
| `store_get_latest_digest` | Latest digest body + pending approvals | **Done** (chat_loop only) |
| `store_list_pending_approvals` | Approvals tab data without digest | Planned |
| `store_list_flaky` | Top flaky tests from local ledger | Planned |
| `store_get_oncall_handoff` | Handoff markdown from store | Planned (optional) |

Dispatch today lives inline in `chat_loop.rs::execute_readonly_tool`. Target: `src/agent/harness_tools.rs` shared by chat and (optionally) workflows.

---

## Config sketch

```yaml
tools:
  base:
    mcp:
      - pr_get_overview
      - pr_get_merge_blockers
      - pr_list_waiting_review
      - pr_list_open
      - ci_get_run_summary
      - issue_list_open
      - alert_list_open
    harness:
      - store_get_latest_digest
      - store_list_pending_approvals
      - store_list_flaky

chat:
  # If omitted: merge(tools.base.*) + workflow defaults
  preferred_tools: []
```

**Merge rules:**

1. `chat.preferred_tools` explicit list → use as-is (override).
2. Else → `tools.base.mcp` + `tools.base.harness` + built-in defaults (current `default_chat_preferred_tools()`).
3. Workflows do not auto-inject into LLM chat; they load `skills/_base/TOOLS.md` for documentation and human consistency.

---

## Skill composition

```
skills/
  _base/
    TOOLS.md          # canonical table: name, layer, when to use
  ci-triage/
    SKILL.md          # technique — composed into agent prompts
  digest-style/
    SKILL.md
  ...

agents/
  daily-work/
    AGENT.md          # task SSOT; references skills[] in frontmatter
```

`load_skill_with_base(path)` (in `engine/skill.rs`):

1. Read `skills/_base/TOOLS.md` via `read_base_tools()`.
2. Read technique skill body.
3. Append `## Base tools` section.

`load_agent_with_base()` applies the same for workflow agents (used by `load_workflow_spec`). Chat uses `load_agent()` + separate `tools_doc` in `PromptBundle` to avoid duplication.

---

## Rust implementation phases

### Phase 1 — Extract harness dispatch (small)

- [ ] Add `src/agent/harness_tools.rs` with `execute_harness_tool(store, name, args)`.
- [ ] Move `store_get_latest_digest` + `format_store_latest_digest` from `chat_loop.rs`.
- [x] Add `skills/_base/TOOLS.md`.
- [x] Add `load_skill_with_base()` / `read_base_tools()` in `engine/skill.rs`.

### Phase 2 — Config wiring

- [ ] `ToolsConfig { base: BaseToolsConfig }` in `config.rs`.
- [ ] Merge into `chat.preferred_tools` at config load time.
- [ ] Document in `coworker.example.yaml` and README.

### Phase 3 — More harness tools

- [ ] `store_list_pending_approvals` — `store.list_pending_approvals()` compact text.
- [ ] `store_list_flaky` — `store.list_flaky_tests()` top N.
- [ ] Register in harness + `_base/TOOLS.md` + default preferred list.

### Phase 4 — Optional MCP

- [ ] `repo_get_info` in unistar-mcp if chat repeatedly needs repo metadata.
- [ ] Add to `tools.base.mcp` defaults.

---

## Chat vs workflow usage

| Consumer | How tools are chosen |
|----------|----------------------|
| **Chat** | LLM JSON `action: tool` → `execute_readonly_tool` → MCP or harness |
| **Workflows** | Rust code calls MCP helpers directly; may adopt harness for store reads |
| **Skills** | Document preferred tools; workflows follow skill tables |

Chat **preferred_tools** is a prompt whitelist — the model is nudged toward base tools but may still use lazy `tool_call` for rare MCP tools.

---

## Mutating tools (explicitly not in base)

Always `action: approval` in chat; never in `tools.base`:

- `ci_rerun_workflow`
- `pr_create_backport`
- `pr_post_comment`

---

## Testing

- Unit tests per harness tool (mock Store).
- Config merge tests for `preferred_tools` defaults vs override.
- Integration: chat turn with `store_get_latest_digest` + one MCP tool (existing MCP test harness).

---

## Related docs

- `design.md` — Chat mode, Top 5 tools, Store entities
- `agents/chat/AGENT.md` — chat agent behavior; tools in `_base/TOOLS.md` + config
- unistar-mcp `pkg/server/` — MCP tool implementations
