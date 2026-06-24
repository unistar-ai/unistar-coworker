# unistar-coworker

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](./LICENSE)
[![Rust 1.75+](https://img.shields.io/badge/rust-1.75%2B-orange.svg)](https://www.rust-lang.org/)

**本地 GitHub 运维秘书** — TUI、Web UI、进程内 GitHub harness、可选 MCP 联邦、本地 LLM。

[English](./README.md) · [中文](./README_CN.md)

---

unistar-coworker 监视 PR 与 CI、分类失败原因、生成 digest，并将 mutating 操作放入 **人工审批** 队列。默认定位是 **运维秘书**，不是无约束的 coding agent：不自动 merge、不自动 push 修复、不替代 GitHub Actions。**Chat** 仍可在工作区内使用 `read_file`、`grep`、`bash_run` 等做轻量本地开发。

GitHub/CI 在 Rust 进程内执行（[`GithubHarness`](./src/github/harness.rs) → `gh` CLI）。可选第三方 MCP（Slack、filesystem、HTTP 网关）通过 `mcp.servers[]` 挂载（stdio 或 Streamable HTTP）。

## 目录

- [功能](#功能)
- [快速开始](#快速开始)
- [安装](#安装)
- [使用](#使用)
- [配置](#配置)
- [MCP 联邦](#mcp-联邦)
- [架构](#架构)
- [开发](#开发)
- [贡献](#贡献)
- [相关](#相关)
- [许可证](#许可证)

## 功能

- **Workflow** — `daily-work`（晨间 triage digest）、`review-radar`（CI 已绿、等 review）；支持 cron、daemon、单次运行
- **Chat** — TUI / CLI / Web 自然语言；LLM 编排 GitHub harness、工作区工具与联邦 MCP
- **GithubHarness** — 进程内 `gh` 调用 GitHub/CI 工具；payload 有 cap；GitHub 不依赖 MCP 子进程
- **MCP 联邦** — `mcp.servers[]` 支持 stdio + HTTP、lazy 发现、mutating 审批、取消进行中的调用
- **安全** — rerun CI、backport、发 comment、MCP mutating 须经 TUI/Web 审批（除非 `chat.auto_approve_mutations`）
- **TUI** — Dashboard、PR 列表、审批、日志、配置、flaky、release、issues、全屏聊天
- **Web UI** — `serve` 浏览器聊天、会话、明暗主题、带来源标签的流式工具/reasoning 卡片
- **Store** — JSON（默认）或 SQLite：digest、快照、flaky 账本、聊天会话、审计日志

## 快速开始

```bash
cd unistar-coworker
cargo build --release
cp coworker.example.yaml coworker.yaml
# 编辑 repos、github:、llm.base_url / model

export GH_TOKEN=ghp_...   # 或 gh auth login

cargo run --release                              # TUI + cron
cargo run --release -- serve                     # Web → http://127.0.0.1:8787
cargo run --release -- run-once                  # 无头 daily-work
cargo run --release -- chat --once "汇总 acme/widget 的 open PR"
```

## 安装

| 依赖 | 用途 |
|------|------|
| **Rust 1.75+** | 编译 |
| **`gh` CLI** | GitHub harness；`gh auth login` 或 `GH_TOKEN` |
| **Ollama / OpenAI 兼容 API**（可选） | `llm.base_url` 指向本地或兼容端点 |

```bash
cargo build --release
# 二进制：target/release/unistar-coworker
```

[unistar-mcp](../unistar-mcp) 是独立 GitHub MCP 产品（Go）；coworker **运行时不需要** 也 **不会启动** 它。

## 使用

### TUI（默认）

```bash
cargo run --release
```

| 键 | 页 |
|----|-----|
| `0` / `?` | Chat |
| `1` | Dashboard |
| `2` | PR 列表 |
| `3` | 审批（`y` / `n`） |
| `4` | 日志 |
| `5` | 配置（github + `mcp[id]` 状态） |
| `6` | Flaky |
| `7` | Release |
| `8` | Issues |

`Tab` / `Shift+Tab` 切换标签 · `r` 跑 daily-work · `q` 退出 · `Esc` 取消当前 chat 轮次。

### Web UI

```bash
cargo run --release -- serve
# 打开 http://127.0.0.1:8787
```

流式聊天、工具/reasoning 卡片（含 `github` / `mcp:…` 来源标签）、上下文面板、审批弹窗、主题切换。静态资源：`src/web/static/`（改 UI 后需重新编译）。

### Chat

```bash
cargo run --release -- chat
cargo run --release -- chat --once "acme/widget #42 CI 为什么红？"
cargo run --release -- chat --session <uuid>
```

GitHub / MCP mutating 工具进 **审批** 队列（除非 `chat.auto_approve_mutations: true`）。

| `chat.tool_mode` | 行为 |
|------------------|------|
| `auto`（默认） | Skill 链 + `tool_search` / `tool_call`；schema 按会话缓存 |
| `lazy` | 同上，尽量少占 upfront 上下文 |
| `native` | 一次性暴露完整 tool schema |

**工作区工具：** `read_file`、`grep`、`glob`、`edit_file`、`write_file`、`bash_run`、`python_run`、`web_fetch`。文件/bash mutating 走 LLM 安全审查；GitHub/MCP mutating 走人工审批。

### Workflow

| Workflow | 说明 | 默认 skills |
|----------|------|-------------|
| `daily-work` | 晨间 PR/CI triage → digest + flaky 账本 | `ci-triage`, `digest-style` |
| `review-radar` | CI 已绿、等待 review 的 PR | `pr-merge`, `digest-style` |

```bash
cargo run --release -- run-once
cargo run --release -- run-once --workflow review-radar
cargo run --release -- daemon          # 仅 cron
cargo run --release -- --attach        # 附着到已运行的 daemon store
```

### 常用命令

| 命令 | 说明 |
|------|------|
| 默认 | TUI + cron |
| `serve` | Web UI + API + WebSocket |
| `--attach` | TUI 附着 daemon |
| `run-once [--workflow ID]` | 无头 workflow（默认 `daily-work`） |
| `daemon` | 仅 cron，无 TUI |
| `chat [--once MSG] [--session UUID]` | 交互或单次聊天 |
| `triage-pr --repo O/R --pr N` | 单 PR triage 调试 |
| `report flaky [--since-days 30]` | 导出 flaky 账本 |
| `store migrate --from json --to sqlite` | 迁移 store |
| `skills list` / `workflows list` | 目录 |

### GitHub 工具

PR：`pr_list_open`、`pr_get_overview`、`pr_get_status`、`pr_get_diff`、`pr_list_changed_files`、`pr_diff_risk_scan`、`pr_create_backport` …

CI：`ci_analyze_pr_failures`、`ci_get_run_summary`、`ci_get_failed_logs`、`ci_rerun_workflow` …

Meta：`tool_search`、`tool_list`、`tool_describe`、`tool_call`、`resource_read`（`github://`、`pr://`、`ci://`）。

实现：[`src/github/harness.rs`](./src/github/harness.rs)。工具名 SSOT：[`skills/_base/TOOLS.md`](./skills/_base/TOOLS.md) + [`src/agent/tool_catalog.rs`](./src/agent/tool_catalog.rs)。

## 配置

从 [coworker.example.yaml](./coworker.example.yaml) 复制。加载路径：当前目录或 `~/.config/unistar-coworker/coworker.yaml`（已 gitignore）。

```yaml
repos:
  - acme/widget

github:
  gh_command: gh
  timeout_secs: 120

llm:
  base_url: http://localhost:11434/v1
  model: your-model
  context_limit: 64000

workflows:
  daily-work: {}
  review-radar: {}

chat:
  workspace: .
  tool_mode: auto   # auto | lazy | native

web:
  bind: 127.0.0.1:8787

theme: dark

policy:
  auto_rerun_flaky: false
```

| 配置项 | 作用 |
|--------|------|
| `github:` | 进程内 harness（`gh_command`、`env`、`timeout_secs` 等） |
| `mcp.servers[]` | 可选第三方 MCP — 见下文 |
| `chat.prompt` | Chat system prompt（默认 `prompts/chat.md`；旧字段 `chat.agent` 仍可用） |
| `chat.skills` | 覆盖 skill 列表（否则用 prompt frontmatter 的 `skills:`） |
| `workflows.<id>.skills` | 覆盖 workflow 默认 skills |

## MCP 联邦

GitHub **永远**走 `github:` / `GithubHarness`。Slack、filesystem 等外部工具走 `mcp.servers[]`：

| 主题 | 行为 |
|------|------|
| 传输 | `stdio`（子进程 JSON-RPC）或 `http`（Streamable HTTP + Bearer） |
| 工具名 | 扁平前缀，如 `slack_post_message` |
| 发现 | 联邦 `tool_list` / `tool_search` / `tool_describe` |
| Mutating | `approval.mutating: required` → 与 `ci_rerun_workflow` 相同审批流 |
| 资源 | `resource_read` 支持 `mcp+{server_id}://…` |
| UI | Config 页 `mcp[id]: ok (N tools)`；工具卡 `mcp:slack · post_message` |
| 热重载 | Web/TUI **Re-probe** 重读配置并重连 |
| 取消 | Chat 取消时 HTTP abort、stdio 杀子进程 |

```yaml
mcp:
  defaults:
    timeout_secs: 120
    startup: on_demand
  servers:
    - id: slack
      transport: stdio
      command: npx
      args: ["-y", "@modelcontextprotocol/server-slack"]
      env:
        SLACK_BOT_TOKEN: ${SLACK_BOT_TOKEN}
      expose:
        prefix: slack_
      approval:
        mutating: required
    - id: ops
      transport: http
      url: http://127.0.0.1:9090/mcp
      headers:
        Authorization: Bearer ${OPS_MCP_TOKEN}
```

实现：[`src/mcp/`](./src/mcp/)。

## 架构

```
┌──────────────────────────────────────────────────────────────────┐
│  unistar-coworker (Rust)                                         │
│  TUI / Web → Engine / Scheduler → Prompts + Skills → Store         │
│                    ↓ LLM              ↓ Approvals                │
│  GithubHarness (进程内 gh) + McpPool (可选 MCP)                  │
└──────────────────────────────────────────────────────────────────┘
```

| 入口 | 触发 | 编排 |
|------|------|------|
| **Workflow** | cron、`run-once`、TUI `r` | 固定 harness 循环 + skills → digest/store |
| **Chat** | TUI `[0]`、`chat`、Web | `prompts/chat.md` + skills + LLM 工具循环 |

### 产品边界

| 是 | 不是 |
|----|------|
| 读取经 cap 的 GitHub/CI 信号，本地 LLM 辅助判断与摘要 | GitHub Actions / CI runner 的替代品 |
| 记账、digest、草稿、审批门控的 mutating 操作 | 无审批自动 merge 或大范围改仓库 |
| TUI + Web 做运维；终端优先 | 托管 SaaS 控制台 |
| **Workflow** 批处理 + **Chat** 临时问答与轻量 coding | 必须用 `unistar-mcp` 子进程才能跑 GitHub |

**刻意不做：** 无审批自动 merge；全库语义 RAG；Workflow 默认不调用第三方 MCP（Chat 在配置后可调用）。

### Skill / Prompt / Harness

| 层 | 路径 | 职责 |
|----|------|------|
| **Skill** | `skills/*/SKILL.md` | 可复用技巧 — triage 规则、语气、digest 格式 |
| **Prompt** | `prompts/chat.md` | Chat system prompt；frontmatter `skills:` 指定默认技巧 |
| **Harness** | `src/agent/`、`src/engine/` | 确定性 Rust — 调度、MCP 池、审批、循环 |

详见 [AGENTS.md](./AGENTS.md)、[skill-agent-harness.md](./skill-agent-harness.md)。

## 开发

```bash
cargo check
cargo clippy -- -D warnings
cargo test
```

```
unistar-coworker/
├── prompts/chat.md
├── skills/
├── src/agent/
├── src/engine/
├── src/github/
├── src/mcp/
├── src/llm/
├── src/tui/
├── src/web/
└── src/store/
```

版本：**1.0.0**（[Cargo.toml](./Cargo.toml)）

## 贡献

请先阅读 [AGENTS.md](./AGENTS.md)：目录结构、harness 约定、PR 期望。Skill/Prompt 与 crate 同仓；工具名须与 `TOOLS.md`、`tool_catalog.rs` 保持一致。

## 相关

- [unistar-mcp](../unistar-mcp) — 独立 GitHub MCP（可选）
- [README.md](./README.md) — English

## 许可证

MIT — 见 [LICENSE](./LICENSE)。
