# MCP recipes

Third-party MCP servers extend chat with external systems. **GitHub always uses the in-process `GithubHarness`** — do not run a separate GitHub MCP for coworker.

See also: [`coworker.example.yaml`](../coworker.example.yaml), README § [MCP federation](../README.md#mcp-federation).

---

## Basics

```yaml
mcp:
  defaults:
    timeout_secs: 120
    lazy: true
    startup: on_demand   # on_demand | eager | disabled
  servers:
    - id: my-server
      enabled: true
      transport: stdio    # or http
      # ...
```

Tool names are prefixed: `slack_post_message`, `filesystem_read_file`, etc.

Third-party MCP is available in **chat** (readonly tools directly; mutating tools via the approval queue unless `approval.mutating: auto`). Reload config with `SIGHUP` or `POST /api/reload`.

---

## Recipe: Slack (stdio)

Post summaries to a channel after triage or on-call review.

**Prereqs:** Slack app with bot token; `npx` on PATH.

```yaml
mcp:
  servers:
    - id: slack
      enabled: true
      transport: stdio
      command: npx
      args: ["-y", "@modelcontextprotocol/server-slack"]
      env:
        SLACK_BOT_TOKEN: ${SLACK_BOT_TOKEN}
      expose:
        prefix: slack_
      approval:
        mutating: required
        tools: [post_message]
      skills: []   # optional: add a slack-ops skill when you author one
```

**Chat example:** “List channels, then draft a one-paragraph CI summary for #eng-oncall” — agent uses `slack_list_channels` (readonly), then `slack_post_message` → **approval**.

---

## Recipe: Filesystem (stdio)

Expose paths outside `chat.workspace` (read-only or read-write depending on server build).

```yaml
mcp:
  servers:
    - id: fs
      enabled: true
      transport: stdio
      command: npx
      args:
        - "-y"
        - "@modelcontextprotocol/server-filesystem"
        - /path/to/allowed/root
      expose:
        prefix: fs_
      approval:
        mutating: required
        tools: [write_file, edit_file]   # adjust to your server's mutating tools
```

Prefer **workspace tools** (`read_file`, `grep`, …) for code under `chat.workspace` — fewer moving parts, parallel reads, LLM safety review on edits.

Use filesystem MCP when the agent must read **fixed host paths** (logs, `/etc`, shared mounts) without widening `chat.workspace`.

---

## Recipe: HTTP / custom ops MCP

Connect to an internal Streamable HTTP MCP (metrics, tickets, runbooks).

```yaml
mcp:
  servers:
    - id: ops
      enabled: true
      transport: http
      url: http://127.0.0.1:9090/mcp
      headers:
        Authorization: Bearer ${OPS_MCP_TOKEN}
      expose:
        prefix: ops_
      approval:
        mutating: required
```

**Health:** `unistar-coworker doctor --json` → `mcp.servers[].connected`, `last_error`. Web Config tab shows the same.

**Security:** keep HTTP MCP on loopback or behind auth; do not expose coworker Web UI to LAN without `web.auth_token`.

---

## Recipe: Per-server skills

When a server's tools are warmed in chat, listed skills auto-load:

```yaml
    - id: slack
      # ...
      skills: [slack-ops]
```

Author `skills/slack-ops/SKILL.md` with `tools:` for common Slack tool chains (see [`skills/_base/SKILL_TEMPLATE.md`](../skills/_base/SKILL_TEMPLATE.md)).

---

## Lazy discovery (`tool_mode: auto`)

Default for 25B+ local models. The agent discovers tools via:

- `skill_load` — from **Available skills** in the system prompt
- `tool_search` / `tool_list_category` / `tool_describe` / `tool_call`

Set `mcp.defaults.lazy: true` to avoid loading every schema at session start.

For VRAM-tight setups, use `chat.tool_mode: lazy` explicitly.

---

## Troubleshooting

| Symptom | Check |
|---------|--------|
| Server `connected: false` | `command`/`args`, env vars, `npx` network; `doctor --json` |
| Tool not found | `expose.prefix` + server tool name; reload after config change |
| Mutating tool silent fail | Approvals queue — Web/TUI or `chat --yes` for headless |
| Stale tools after edit | `SIGHUP` / `POST /api/reload` |
