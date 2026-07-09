# RPC mode — JSONL over stdin/stdout

Pi-style machine protocol for driving `unistar-coworker` from scripts, Slack bots,
or internal dashboards without starting the Web UI.

**Compatibility:** see [STABILITY.md](./STABILITY.md). RPC `op` values marked **Stable** below are not removed or semantically changed except in major releases.

## Start

```bash
unistar-coworker rpc [--session <uuid>] [--yes] [--timeout <secs>]
```

- **`--session`**: resume an existing chat session (auto-created on first `chat` if omitted)
- **`--yes`**: auto-approve mutating tools (no approval pause)
- **`--timeout`**: per-turn wall-clock limit in seconds

Stdout is **only** JSON lines (one object per line). Progress and errors also go to
stdout as typed events so callers can parse a single stream. Tracing logs go to stderr.

## Request format (stdin)

Each non-empty line is one JSON object with an `op` field:

### `chat` — **Stable**

```json
{"op":"chat","message":"triage PR #42 in acme/widget"}
```

Runs one user turn. Streams `progress` lines, then `result` or `error`.

### `get_state` — **Stable**

```json
{"op":"get_state"}
```

Returns a `state` line with a full `WebSnapshot` (same shape as `/api/state`).

### `cancel` — **Stable**

```json
{"op":"cancel"}
```

Cancels the in-flight chat turn. Responds with `{"type":"cancelled"}`.

### `switch_profile` — **Stable**

```json
{"op":"switch_profile","profile":"fast"}
```

Switches the active LLM profile (same as Web `POST /api/config/llm-profile`).

Unknown ops respond with `{"type":"error","code":"unknown_op","op":"..."}`.

## Response format (stdout)

### Progress (streaming)

```json
{"type":"progress","stage":"tool","name":"pr_get_overview","detail":"..."}
```

Stages mirror `ChatProgress` (tool start/done, reasoning, assistant partial, etc.).

### Result

```json
{
  "type": "result",
  "ok": true,
  "session_id": "9950379a-3db7-46ec-98ed-11310014b456",
  "assistant": "…",
  "tool_calls": [{"tool":"pr_get_overview","output":"…"}],
  "awaiting_approval": false
}
```

When a mutating tool needs approval and `--yes` was not passed:

```json
{
  "type": "error",
  "code": "approval_required",
  "error": "awaiting approval",
  "pending_approval": { "tool": "pr_merge", "args": "…", "description": "…" }
}
```

Process exit code **`3`** (`EXIT_APPROVAL`).

### State

```json
{"type":"state","snapshot":{ ... }}
```

### Errors

```json
{"type":"error","code":"bad_request","error":"..."}
{"type":"error","code":"turn_failed","error":"..."}
{"type":"error","code":"profile","error":"..."}
```

## Exit codes

| Code | Meaning |
|------|---------|
| `0` | Success |
| `1` | General / turn error |
| `2` | Config / environment (`doctor` fail, bad config) |
| `3` | Approval required (headless without `--yes`) |
| `4` | Timeout (`--timeout`) |

## Example session

```bash
printf '%s\n' \
  '{"op":"chat","message":"list open PRs on acme/widget"}' \
  '{"op":"get_state"}' \
  | unistar-coworker rpc --yes 2>/dev/null | jq -c .
```

## Related

- **One-shot CLI**: `unistar-coworker chat --once "…" --json` (single message, then exit)
- **Hot reload**: `POST /api/reload` or `SIGHUP` on TUI/serve
- **Session export**: `unistar-coworker export session <id> --format jsonl`
- **Health check**: `unistar-coworker doctor --json` or `GET /api/doctor`
