# Workflows

**Workflows** are scheduled or one-shot **batch** jobs — distinct from interactive **chat**. The product ships two built-in GitHub ops workflows; you customize them with config and skills, not new Rust IDs (today).

Chat covers open-ended tasks (“every Monday summarize this folder”) via cron **outside** coworker or by asking the agent in a long-running `daemon` + manual triggers.

---

## Built-in catalog

Registry SSOT: [`workflow_registry.rs`](../crates/core/src/engine/workflow_registry.rs).

| ID | Description | Default skills |
|----|-------------|----------------|
| `daily-work` | Morning PR/CI triage across `repos:` → digest + flaky ledger | `ci-triage`, `digest-style` |
| `review-radar` | Open PRs with green CI still waiting for review | `pr-merge`, `digest-style` |

```bash
unistar-coworker workflows list
unistar-coworker run-once --workflow daily-work
unistar-coworker run-once --workflow review-radar --json
```

TUI: Dashboard **`r`** runs `daily-work`. Chat harness: `harness_run_workflow`, `harness_daily_digest`.

**Requires:** `repos:` + working `gh` auth for GitHub workflows. Workspace-only configs should disable or omit workflows.

---

## Configuration

```yaml
repos:
  - org/service-a
  - org/service-b

workflows:
  mcp_readonly: false          # global default for third-party MCP in batch runs
  daily-work:
    enabled: true
    schedule: "0 9 * * 1-5"    # cron (daemon / default TUI scheduler)
    skills: []                 # empty = built-in default skills
    mcp_readonly: true         # optional per-workflow override
  review-radar:
    enabled: true
    schedule: "0 14 * * *"
```

| Field | Meaning |
|-------|---------|
| `enabled` | `false` skips scheduler and rejects `run-once` |
| `schedule` | Standard cron; daemon process only |
| `skills` | Replace default technique list (paths under `skills/`) |
| `mcp_readonly` | Allow **readonly** third-party MCP during this workflow |

### MCP policy in batch runs

| Mode | Third-party MCP | Mutating MCP |
|------|-----------------|--------------|
| Default | Blocked | Blocked |
| `mcp_readonly: true` | Readonly tools OK | Still blocked |
| Chat | Full policy + approvals | Approval queue |

GitHub harness tools are always available when `gh` is configured — workflows do not use external GitHub MCP.

---

## How a run works

```
cron / run-once / TUI r
        ↓
WorkflowRunner (config + registry)
        ↓
load_workflow_spec → skills merged into prompt
        ↓
AgentLoop (fixed steps per workflow id)
        ↓
Store: digest, snapshots, flaky ledger, audit
```

`daily-work` triages PRs per repo policy (`policy.max_prs_per_repo`, etc.). `review-radar` lists waiting-review PRs and exports a digest line.

Implementation: [`agent/loop.rs`](../crates/core/src/agent/loop.rs), [`agent/workflow_harness.rs`](../crates/core/src/agent/workflow_harness.rs).

---

## Customizing behavior (no new workflow id)

Today, new workflow **ids** require a Rust registry entry. You can still:

1. **Swap skills** — `workflows.daily-work.skills: [ci-triage, flaky-tests, digest-style]`
2. **Author skills** — [`skills/_base/SKILL_TEMPLATE.md`](../skills/_base/SKILL_TEMPLATE.md)
3. **Chat harness** — `harness_triage_pr` for one PR; `harness_run_workflow` for batch ids
4. **External cron** — `run-once` from systemd/k8s CronJob with different configs
5. **Reports** — `report oncall`, `report ci` (CLI; some need MCP/GitHub)

Example: heavier flake focus on morning digest:

```yaml
workflows:
  daily-work:
    skills: [ci-triage, flaky-tests, digest-style]
```

---

## Scheduler / daemon

```bash
unistar-coworker daemon                  # cron only, no TUI
unistar-coworker daemon --pid-file ./coworker.pid
unistar-coworker tui                     # TUI + same cron in-process
unistar-coworker --attach                # TUI → existing daemon store
```

Graceful shutdown: SIGINT/SIGTERM. Logs via TUI, Web, or store audit.

---

## GitHub ops pack

Skill catalog and workflow defaults for GitHub: [`skills/github-ops-pack/README.md`](../skills/github-ops-pack/README.md).

General agent (workspace) does not require any workflow.
