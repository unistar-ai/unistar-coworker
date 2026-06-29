# Web UI WebSocket Protocol

> Contract for the one-way WebSocket stream from `unistar-coworker serve` to
> the browser SPA. The client never sends data over WS (apart from ping/pong);
> mutations go through REST `POST` handlers.

## Connection

- Endpoint: `GET /ws` (upgrade)
- Auth (when `web.auth_token` is set):
  - `Authorization: Bearer <token>` header, **or**
  - `?token=<token>` query parameter (browsers cannot set headers on `new WebSocket()`)
- On connect, the server sends one **full snapshot** message (no `_type` field).
- Subsequent messages are either `live` or `chat` patches (see below).
- The server does not expect any client frames except protocol-level Ping/Close.

## Message shapes

### 1. Full snapshot (initial state)

No `_type` field. The browser treats the entire JSON as the new `state`.

Fields (see [`src/web/snapshot.rs`](../src/web/snapshot.rs) `WebSnapshot` for the authoritative list):

| Field | Type | Notes |
|-------|------|-------|
| `tab` | string | active tab id |
| `tabs` | string[] | enabled tab ids |
| `status` | string | status bar text |
| `engine_busy` | bool | workflow running |
| `engine_workflow_id` | string? | workflow id when busy |
| `chat_enabled` | bool | chat tab available |
| `chat_busy` | bool | chat turn in progress |
| `chat_session_id` | string? | current session UUID |
| `chat_lines` | string[] | transcript lines |
| `chat_tool_outputs` | map<string,string> | line index → tool output body (expand in UI) |
| `chat_history_revision` | u64 | bumps on history change |
| `chat_context_revision` | u64 | bumps on context change |
| `chat_streaming` | string? | in-progress assistant text |
| `chat_reasoning` | string? | in-progress reasoning text |
| `chat_tool_running` | string? | tool currently executing |
| `chat_tool_running_detail` | string? | extra detail for running tool |
| `chat_tool_pending` | string? | tool queued for approval |
| `chat_turn_phase` | string? | derived: `model`/`tool`/`streaming`/`reasoning`/`summarizing`/`activity` (null when not busy) |
| `chat_reasoning_compressing` | bool | context summarization in progress |
| `chat_activity_flow` | {kind,text}? | activity flow card |
| `chat_context_visible` | bool | context pane open |
| `chat_context` | object | context panel payload (stats, tools, skills, messages) |
| `chat_pending_approval` | object? | pending approval metadata |
| `approval_dialog` | object? | approval modal payload |
| `digest_history` | object[] | dashboard digest list |
| `digest_bodies` | map<string,string> | date → digest markdown |
| `selected_digest_date` | string? | |
| `prs` | object[] | PR list |
| `pr_filter` / `pr_sort` | string | |
| `selected_pr_index` | usize | |
| `pr_overview` | string? | selected PR overview markdown |
| `pr_overview_loading` | bool | |
| `approvals` | object[] | pending approvals |
| `log_filter` | string | |
| `logs` | object[] | recent log entries (≤ 200) |
| `config_path` | string | |
| `repos` | string[] | |
| `llm_model` | string | |
| `github_ok` / `llm_ok` | bool | connectivity probes |
| `github_latency_ms` / `llm_latency_ms` | u128? | |
| `mcp_servers` | object[] | per-server status |
| `attach_mode` | bool | TUI attached to daemon store |
| `auto_approve_mutations` | bool | `chat.auto_approve_mutations` |
| `ui_theme` | string | `dark` / `light` |

### 2. Live patch (`_type: "live"`)

High-frequency (~100ms throttle) patch for streaming/tool progress. Avoids
re-sending history/context/digest/PR/log payload.

| Field | Type |
|-------|------|
| `_type` | `"live"` |
| `status` | string |
| `chat_busy` | bool |
| `chat_streaming` | string? |
| `chat_reasoning` | string? |
| `chat_tool_running` | string? |
| `chat_tool_running_detail` | string? |
| `chat_tool_pending` | string? |
| `chat_turn_phase` | string? |
| `chat_reasoning_compressing` | bool |
| `chat_activity_flow` | {kind,text}? |

Client applies these into `state` and triggers a **live-only render** (no
history/context sync).

### 3. Chat patch (`_type: "chat"`)

Lower-frequency patch that includes history/context/approval changes plus the
live fields. Tool output bodies are truncated to **8,000 chars** (full snapshot
is not truncated).

| Field | Type |
|-------|------|
| `_type` | `"chat"` |
| `status` | string |
| `chat_busy` | bool |
| `chat_session_id` | string? |
| `chat_lines` | string[] |
| `chat_tool_outputs` | map<string,string> | truncated to 8k chars per body |
| `chat_history_revision` | u64 |
| `chat_context_revision` | u64 |
| `chat_streaming` | string? |
| `chat_reasoning` | string? |
| `chat_tool_running` | string? |
| `chat_tool_running_detail` | string? |
| `chat_tool_pending` | string? |
| `chat_turn_phase` | string? |
| `chat_reasoning_compressing` | bool |
| `chat_activity_flow` | {kind,text}? |
| `chat_context_visible` | bool |
| `chat_context` | object |
| `chat_pending_approval` | object? |
| `approval_dialog` | object? |

Client applies these and triggers a **structural chat render** (history +
context + live zone).

## Server event loop classification

`spawn_event_loop` in [`src/web/mod.rs`](../src/web/mod.rs) classifies each
`AppEvent` into a snapshot kind:

| `AppEvent` variant | Kind | Effect |
|--------------------|------|--------|
| `ChatProgress(p)` where `p` is live-only (streaming/reasoning/tool/flow) | `Live` | `live_dirty = true` |
| `ChatProgress(_)` (other) | `Chat` | `chat_dirty = true` |
| `ChatReply` | `Chat` | `chat_dirty = true` |
| anything else | `Full` | publish full snapshot immediately |

A 100ms tick coalesces dirty flags: `chat_dirty` wins over `live_dirty` (a
chat patch supersedes a live patch). In `--attach` mode a 2s poll re-hydrates
from the store and publishes a full snapshot.

## Client apply order (`src/web/static/app.js`)

```
ws.onmessage:
  if data._type === "live":  applyLivePatch(data)  → scheduleLiveRender()  (rAF)
  if data._type === "chat":  applyChatPatch(data)  → scheduleChatRender()   (rAF, 120ms debounce when chat_busy)
  else:                      state = data           → scheduleRender()      (rAF, full)
```

`applyLivePatch` merges the 10 live fields into `state`. `applyChatPatch`
merges the 19 chat fields. Both initialize `state = {}` if absent.

## Reconnection

The client reconnects on `ws.onclose` with exponential backoff
(1s → 2s → 4s → 8s → 16s → cap 30s). On reconnect it refetches `/api/state`
to recover any patches missed during the disconnect.

## Adding / removing a patch field — checklist

1. **Rust**: update `WebLivePatch` / `WebChatPatch` in
   [`src/web/snapshot.rs`](../src/web/snapshot.rs) and the corresponding
   `build_*_from` builder.
2. **Rust tests**: update `EXPECTED_LIVE_PATCH_KEYS` / `EXPECTED_CHAT_PATCH_KEYS`
   in `snapshot::tests` (the contract test will fail until you do).
3. **JS**: update `applyLivePatch` / `applyChatPatch` in
   [`src/web/static/app.js`](../src/web/static/app.js) to read the new field.
4. **Docs**: update the table above.

The contract test (`live_patch_serializes_expected_keys` /
`chat_patch_serializes_expected_keys`) is the single source of truth for the
field set — any drift fails CI.
