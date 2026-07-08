# unistar-coworker

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](./LICENSE)
[![Rust 1.75+](https://img.shields.io/badge/rust-1.75%2B-orange.svg)](https://www.rust-lang.org/)
[![CI](https://github.com/unistar-ai/unistar-coworker/actions/workflows/ci.yml/badge.svg)](./.github/workflows/ci.yml)

**本地 GitHub 运维秘书** — 终端优先的 TUI、浏览器 Web UI、进程内 GitHub harness、可选 MCP 联邦、本地 LLM。监视 PR 与 CI、分类失败原因、生成 digest，并将所有 mutating 操作放入 **人工审批** 队列。

[English](./README.md) · [中文](./README_CN.md)

---

## 概览

`unistar-coworker` **不是**无约束的 coding agent，也**不是** GitHub Actions 的替代品。它是一个运维秘书，负责：

- 运行定时 **workflow**（`daily-work` triage、`review-radar`）与交互式 **chat** 模式，用于临时问答与轻量本地开发。
- GitHub/CI 在 Rust 进程内执行（[`GithubHarness`](./crates/core/src/github/harness.rs) → `gh` CLI）— GitHub 不依赖 MCP 子进程。
- 通过 `mcp.servers[]` 挂载可选 **第三方 MCP**（Slack、filesystem、HTTP 网关，stdio 或 Streamable HTTP）。
- 使用 **本地 LLM**（Ollama / OpenAI 兼容）做分类、chat 编排与 digest。
- **绝不**自动执行 mutating 操作 — rerun CI、backport、发 comment、MCP mutating 工具全部走 TUI/Web 审批队列，除非显式开启 `chat.auto_approve_mutations`。

Chat 仍可在工作区内使用 `read_file`、`grep`、`bash_run` 等做轻量本地编码；文件/bash mutating 走 LLM 安全审查，GitHub/MCP mutating 走人工审批。

---

## 目录

- [功能](#功能)
- [快速开始](#快速开始)
- [依赖](#依赖)
- [使用](#使用)
  - [TUI](#tui)
  - [Web UI](#web-ui)
  - [Chat](#chat)
  - [Workflow](#workflow)
  - [常用命令](#常用命令)
- [配置](#配置)
- [存储](#存储)
- [MCP 联邦](#mcp-联邦)
- [架构](#架构)
- [开发](#开发)
  - [快速编译（日常开发）](#快速编译日常开发)
  - [Web UI 开发（HMR）](#web-ui-开发hmr)
  - [Web UI E2E（Playwright）](#web-ui-e2eplaywright)
  - [Feature flags](#feature-flags)
- [项目结构](#项目结构)
- [贡献](#贡献)
- [相关](#相关)
- [许可证](#许可证)

---

## 功能

| 领域 | 能力 |
|------|------|
| **Workflow** | `daily-work`（晨间 PR/CI triage → digest + flaky 账本）、`review-radar`（CI 已绿、等 review）；支持 cron、daemon、单次运行 |
| **Chat** | TUI / CLI / Web 自然语言；LLM 编排 GitHub harness、工作区工具与联邦 MCP |
| **GithubHarness** | 进程内 `gh` 调用 GitHub/CI 工具；payload 有 cap；GitHub 不依赖 MCP 子进程 |
| **MCP 联邦** | `mcp.servers[]` 支持 stdio + HTTP、lazy 发现、mutating 审批、per-server skills、取消进行中的调用 |
| **安全** | rerun CI、backport、发 comment、MCP mutating 须经 TUI/Web 审批（除非 `chat.auto_approve_mutations` 或 per-server `approval.mutating: auto`） |
| **TUI** | Dashboard、PR 列表、审批、日志、配置、flaky、release、issues、全屏聊天 |
| **Web UI** | `serve` 浏览器聊天、会话、明暗主题、带来源标签的流式工具/reasoning 卡片、审批弹窗、Markdown 导出 |
| **Store** | JSON（默认）或 SQLite：digest、快照、flaky 账本、聊天会话、审计日志；`store migrate` 与 `store compact` 命令 |

---

## 快速开始

```bash
cd unistar-coworker
unistar-coworker init --repos acme/widget --llm-url http://localhost:11434/v1
# 或：cp coworker.example.yaml coworker.yaml 后手动编辑

export GH_TOKEN=ghp_...   # 或 gh auth login

unistar-coworker doctor          # 检查 config / gh / LLM / MCP / store

# 前端：构建一次（开发从磁盘 serve；发布嵌入二进制）
(cd web-ui && npm install && npm run build:fast)

cargo build --release --features embed-web-ui

./target/release/unistar-coworker                              # TUI + cron
./target/release/unistar-coworker serve                        # Web → http://127.0.0.1:8787
./target/release/unistar-coworker run-once                     # 无头 daily-work
./target/release/unistar-coworker chat --once "汇总 acme/widget 的 open PR" --json
```

---

## 依赖

| 依赖 | 用途 |
|------|------|
| **Rust 1.75+**（工具链 `stable`） | 编译 `unistar-coworker` |
| **`gh` CLI** | GitHub harness；`gh auth login` 或 `GH_TOKEN` |
| **Ollama / OpenAI 兼容 API**（可选） | `llm.base_url` 指向本地或兼容端点；离线时 chat/triage 降级为启发式 |

```bash
# 发布 / 部署（单二进制，内嵌 Web UI）
cargo build --release --features embed-web-ui
# 二进制：target/release/unistar-coworker

# 开发（更快 — 运行时从 web-ui/dist/ 读取；见「开发」）
cargo build
# 二进制：target/debug/unistar-coworker
```

> [unistar-mcp](../unistar-mcp) 是独立 GitHub MCP 产品（Go）；coworker **运行时不需要** 也**不会启动**它 — GitHub 始终走进程内 `GithubHarness`。

---

## 使用

### TUI

默认命令启动终端 UI 并附带 cron 调度器。

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

Web UI 是 **React 18 SPA**（Vite + Tailwind + Radix UI + zustand），提供流式聊天（含工具/reasoning 卡片）、上下文面板、审批弹窗、**LLM profile 切换**（Config 页）、**分支 regenerate**、主题切换与转录导出。源码在 `web-ui/`。

**资源如何提供：**

| 构建 | 命令 | Web UI 交付方式 |
|------|------|-----------------|
| **开发**（默认） | `cargo build` / `cargo run -- serve` | 运行时从 `web-ui/dist/` 读盘 — 只改 Rust 不会重新嵌入 JS |
| **发布 / CI** | `cargo build --release --features embed-web-ui` | `build.rs` 通过 `include_str!` / `include_bytes!` 嵌入，单文件部署 |

`build.rs` **不会**跑 `npm` — 前端构建由开发者、CI 或 [`scripts/start-agent.sh`](./scripts/start-agent.sh) 负责（`npm run build:fast`）。启用 `embed-web-ui` 时，manifest 按内容去重，只有 bundle 真变才触发重编译。未启用时需先 `npm run build:fast`，否则 React 路由返回 503（legacy UI 可能仍可用）。

**热重载**（无需重启进程）：向运行中的 `serve` / `tui` / `daemon` 发 `SIGHUP`，或 `POST /api/reload` — 重载 `coworker.yaml`、skills、prompts 与 MCP 连接。

**健康检查 API：** `GET /api/doctor` 与 `unistar-coworker doctor --json` 返回相同 JSON（config、`gh`、LLM、MCP、store）。

**开发期 HMR：**

```bash
# 终端 1：Rust 后端
cargo run -- serve

# 终端 2：Vite dev server（热重载，代理 /api 与 /ws 到 :8787）
cd web-ui && npm install && npm run dev
# 打开 http://localhost:5173
```

**安全模型。** Web UI 面向**本机可信环境**。建议保持 `web.bind` 为默认 `127.0.0.1:8787`，避免聊天、审批、workflow 暴露到局域网。

当必须绑定到非本机（例如 `0.0.0.0`）时，请设置 `web.auth_token`：

- **静态资源**（`/`、`/assets/*`）保持公开，便于浏览器作为子资源加载。它们不含密钥 — 仅暴露 UI 结构。
- **敏感路由**需要鉴权：所有 `/api/*`（除 `/api/health`）与 `/ws` WebSocket 升级。
- 接受两种鉴权方式：
  - `Authorization: Bearer <token>` 请求头（API 客户端、curl 首选）。
  - `?token=<token>` query 参数（用于 `new WebSocket()`，它无法设置请求头）。
- 浏览器 UI 首次加载时读取 `?token=`，存入 `sessionStorage`，从 URL 中剥离，并自动注入到后续每个 fetch 与 WebSocket 请求。
- `/api/health` 保持无鉴权，方便外部健康探活。
- 每个响应附带严格的 **Content-Security-Policy** 头：`script-src 'self'`（禁止内联脚本）、`object-src 'none'`、`frame-ancestors 'none'`、`connect-src 'self' ws: wss:`。

> `?token=` 形式可能出现在服务器日志或浏览器历史中；如需更强安全，建议用反向代理注入 auth cookie。本地开发保持 `auth_token` 未设置即可。

### Chat

```bash
cargo run --release -- chat
cargo run --release -- chat --once "acme/widget #42 CI 为什么红？"
cargo run --release -- chat --session <uuid>
cargo run --release -- chat --list-sessions
```

GitHub / MCP mutating 工具进 **审批** 队列（除非 `chat.auto_approve_mutations: true`）。

| `chat.tool_mode` | 行为 |
|------------------|------|
| `auto`（默认） | Skill 链 + `tool_search` / `tool_call`；schema 按会话缓存 |
| `lazy` | 同上，尽量少占 upfront 上下文 |
| `native` | 一次性暴露完整 tool schema |

**工作区工具：** `read_file`、`grep`、`glob`、`edit_file`、`write_file`、`bash_run`、`python_run`、`web_fetch`。文件/bash mutating 走 LLM 安全审查；GitHub/MCP mutating 走人工审批。

**韧性参数**（可选）：

- `chat.llm_step_timeout_secs` — 单个 LLM step 的墙钟超时（0 = 不限）。
- `chat.reasoning_only_warn_secs` — 当只有 reasoning 增长、无可见 content 时停止流（0 = 关闭）。避免 reasoning-only 模型空转 90s。

### Workflow

| Workflow | 说明 | 默认 skills |
|----------|------|-------------|
| `daily-work` | 晨间 PR/CI triage → digest + flaky 账本 | `ci-triage`, `digest-style` |
| `review-radar` | CI 已绿、等待 review 的 PR | `pr-merge`, `digest-style` |

```bash
cargo run --release -- run-once
cargo run --release -- run-once --workflow review-radar
cargo run --release -- daemon          # 仅 cron
cargo run --release -- --attach        # 附着到已运行 daemon 的 store
```

批处理 workflow **默认不调用第三方 MCP**；设置 `workflows.mcp_readonly: true`（全局）或 `workflows.<id>.mcp_readonly: true`（单 workflow）可放开只读 MCP。Mutating MCP 始终仅限 chat。

### 常用命令

| 命令 | 说明 |
|------|------|
| 默认 | TUI + cron |
| `serve [--bind ADDR]` | Web UI + API + WebSocket |
| `--attach` | TUI 附着 daemon |
| `run-once [--workflow ID]` | 无头 workflow（默认 `daily-work`） |
| `daemon` | 仅 cron，无 TUI |
| `chat [--once MSG] [--session UUID] [--list-sessions]` | 交互或单次聊天 |
| `triage-pr --repo O/R --pr N` | 单 PR triage 调试 |
| `report oncall` | 本地 store 生成的 on-call 交接包（无需 MCP） |
| `report ci [--since-days 7]` | CI 效率报告（需 MCP） |
| `store migrate --from json --to sqlite --source DIR --dest FILE` | 迁移 store 后端 |
| `store compact [--audit-days 90] [--digest-keep 30] [--workflow-runs-days 30]` | 清理旧审计、digest、workflow 运行记录 |
| `skills list` / `workflows list` | 打印目录 |

### GitHub 工具

PR：`pr_list_open`、`pr_get_overview`、`pr_get_status`、`pr_get_diff`、`pr_list_changed_files`、`pr_diff_risk_scan`、`pr_create_backport` …

CI：`ci_analyze_pr_failures`、`ci_get_run_summary`、`ci_get_failed_logs`、`ci_rerun_workflow` …

Meta：`tool_search`、`tool_list`、`tool_describe`、`tool_call`、`resource_read`（`github://`、`pr://`、`ci://`）。

实现：[`crates/core/src/github/harness.rs`](./crates/core/src/github/harness.rs)。工具名 SSOT：[`skills/_base/TOOLS.md`](./skills/_base/TOOLS.md) + [`crates/core/src/agent/tool_catalog.rs`](./crates/core/src/agent/tool_catalog.rs)。

---

## 配置

从 [coworker.example.yaml](./coworker.example.yaml) 复制。加载路径：当前目录或 `~/.config/unistar-coworker/coworker.yaml`（均已 gitignore）。

```yaml
repos:
  - acme/widget

github:
  gh_command: gh
  timeout_secs: 120
  # tool_timeouts:
  #   ci_get_failed_logs: 180

llm:
  base_url: http://localhost:11434/v1
  model: your-model
  context_limit: 64000
  # api_key: ollama

workflows:
  # mcp_readonly: false   # 全局默认 — 批处理 workflow 不调用第三方 MCP
  daily-work: {}
  review-radar: {}

chat:
  workspace: .
  tool_mode: auto        # auto | lazy | native
  # llm_step_timeout_secs: 180
  # reasoning_only_warn_secs: 30
  # bash: { timeout_secs: 30, max_output_chars: 16000 }
  # python: { timeout_secs: 30, max_output_chars: 16000, command: python3 }
  # web_fetch:
  #   timeout_secs: 30
  #   max_content_chars: 32000
  #   allow_localhost: true
  #   browser_timeout_secs: 60
  #   chromium_path: /Applications/Google Chrome.app/Contents/MacOS/Google Chrome

web:
  bind: 127.0.0.1:8787
  # auth_token: your-secret   # 非本机绑定时必填；保护静态资源、/api/*、/ws

theme: dark   # dark | light | none（Web 把 none 当作 dark）

policy:
  auto_rerun_flaky: false
```

| 配置项 | 作用 |
|--------|------|
| `github:` | 进程内 harness（`gh_command`、`env`、`timeout_secs`、`tool_timeouts`） |
| `mcp.servers[]` | 可选第三方 MCP — 见 [MCP 联邦](#mcp-联邦) |
| `chat.prompt` | Chat system prompt（默认 `prompts/chat.md`，编译期内嵌；自定义路径从磁盘加载） |
| `chat.skills` | 覆盖 skill 列表（否则用 prompt frontmatter 的 `skills:`） |
| `chat.tool_mode` | 工具发现策略 — 见 [Chat](#chat) |
| `chat.auto_approve_mutations` | 跳过 mutating 工具审批队列（默认 `false`） |
| `web.bind` | `serve` 监听地址（默认 `127.0.0.1:8787`） |
| `web.auth_token` | 非本机绑定时保护静态资源、`/api/*`、`/ws` 的 Bearer 令牌 |
| `workflows.<id>.skills` | 覆盖 workflow 默认 skills |
| `workflows.mcp_readonly` | 全局默认：批处理 workflow 是否允许只读第三方 MCP（默认 `false`） |
| `workflows.<id>.mcp_readonly` | 单 workflow 覆盖；mutating MCP 始终仅限 chat |
| `policy.auto_rerun_flaky` | 自动 rerun flaky CI（默认 `false`；否则需审批门控） |

---

## 存储

默认使用 `./data` 下的 JSON 后端（已 gitignore）。长期运行 `serve` / `daemon` 或多会话场景建议改用 **SQLite** — 单文件、并发读与长历史更稳：

```yaml
storage:
  backend: sqlite
  path: ./data/coworker.db
```

从已有 JSON 迁移：

```bash
cargo run --release -- store migrate --from json --to sqlite \
  --source ./data --dest ./data/coworker.db
```

定期清理保持 store 紧凑：

```bash
cargo run --release -- store compact            # 默认：审计 90d、保留 30 个 digest、workflow 运行 30d
cargo run --release -- store compact --audit-days 180 --digest-keep 60
```

---

## MCP 联邦

GitHub **永远**走进程内 `GithubHarness`。Slack、filesystem 等外部工具走 `mcp.servers[]`：

| 主题 | 行为 |
|------|------|
| 传输 | `stdio`（子进程 JSON-RPC）或 `http`（Streamable HTTP + Bearer） |
| 工具名 | 扁平前缀，如 `slack_post_message` |
| 发现 | 联邦 `tool_list` / `tool_search` / `tool_describe` |
| Mutating | `approval.mutating: required` → 与 `ci_rerun_workflow` 相同审批流（`ApprovalKind::McpTool`） |
| 资源 | `resource_read` 支持 `mcp+{server_id}://…` |
| UI | Config 页 `mcp[id]: ok (N tools)`；工具卡 `mcp:slack · post_message` |
| 热重载 | Web/TUI **Re-probe** 重读配置并重连 |
| Per-server skills | `skills: [name]` 在工具预热时自动加载对应技巧 skill |
| 取消 | Chat 取消时 HTTP abort、stdio 杀子进程 |
| Workflow | 批处理默认不调第三方 MCP；`workflows.mcp_readonly: true` 或单 workflow `mcp_readonly: true` 放开只读（mutating 始终仅限 chat） |

```yaml
mcp:
  defaults:
    timeout_secs: 120
    startup: on_demand      # on_demand | eager | disabled
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
      skills: [slack-ops]
    - id: ops
      enabled: true
      transport: http
      url: http://127.0.0.1:9090/mcp
      headers:
        Authorization: Bearer ${OPS_MCP_TOKEN}
```

实现：[`crates/core/src/mcp/`](./crates/core/src/mcp/)。

---

## 架构

```
┌──────────────────────────────────────────────────────────────────┐
│  unistar-coworker (Rust)                                         │
│  TUI / Web → Engine / Scheduler → Prompts + Skills → Store        │
│                    ↓ LLM              ↓ Approvals                 │
│  GithubHarness (进程内 gh) + McpPool (可选 MCP)                   │
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

三层架构，职责不混用：

| 层 | 路径 | 职责 |
|----|------|------|
| **Skill** | `skills/*/SKILL.md` | 可复用技巧 — triage 规则、语气、digest 格式。不含 cron、不含 harness 逻辑。 |
| **Prompt** | `prompts/chat.md` | Chat system prompt；frontmatter `skills:` 指定默认技巧。编译期内嵌。 |
| **Harness** | `crates/core/src/agent/`、`crates/core/src/engine/` | 确定性 Rust — 调度、MCP 池、审批、token 预算、循环 |

工具名 SSOT：[`skills/_base/TOOLS.md`](./skills/_base/TOOLS.md) + [`crates/core/src/agent/tool_catalog.rs`](./crates/core/src/agent/tool_catalog.rs)。

详见 [AGENTS.md](./AGENTS.md)。

---

## 开发

```bash
# 快速 Rust 迭代（不嵌入前端）
cargo check
cargo check -p coworker-tui    # 仅 TUI 层
cargo clippy --workspace -- -D warnings
cargo test --workspace

# CI / 发布标准（嵌入 web-ui/dist/）
cargo clippy --workspace --features embed-web-ui -- -D warnings
cargo test --workspace --features embed-web-ui

cargo test --workspace --no-default-features   # 不含 headless Chromium 的精简构建
cargo fmt --check
```

### 快速编译（日常开发）

默认 `cargo build` / `cargo check` **不启用** `embed-web-ui`。React UI 在运行时从 `web-ui/dist/` 提供（[`crates/web/src/ui.rs`](./crates/web/src/ui.rs)），改 Rust 不会因前端资源变化而触发整包重嵌入。

Rust 代码已拆为 **Cargo workspace**（`crates/core`、`crates/tui`、`crates/web`、`crates/cli`、`crates/unistar-coworker`）。依赖 crate 未变时，`cargo check -p coworker-tui` 等可跳过无关层的重编译。

```bash
# 一次性（或 web-ui 源码变更后）
cd web-ui && npm install && npm run build:fast

# 后端 — 增量编译，不嵌入 JS
cargo run -- serve          # http://127.0.0.1:8787
```

本地加速选项见 [`.cargo/config.toml`](./.cargo/config.toml)：`debug = 1`、增量编译，以及可选的 `sccache` / `mold`（已注释，安装后取消注释）。

**发布 / 部署**（单二进制、内嵌 UI — 与 CI、[`scripts/start-agent.sh`](./scripts/start-agent.sh) 一致）：

```bash
(cd web-ui && npm run build:fast)
cargo build --release --features embed-web-ui
```

`cargo build` 不依赖 Node。仅在构建 React UI 时需要 Node：

```bash
brew install node          # macOS
cd web-ui && npm install   # 首次
```

### Web UI 开发（HMR）

```bash
# 终端 1：Rust 后端
cargo run -- serve

# 终端 2：Vite dev server（热重载，代理 /api 与 /ws 到 :8787）
cd web-ui && npm run dev
# 打开 http://localhost:5173
```

CI（`.github/workflows/ci.yml`）运行 `cargo fmt --check`、带 `--workspace --features embed-web-ui` 的 `cargo clippy` / `cargo test`（需先构建 `web-ui/dist/`）、`--no-default-features` 构建/测试 job，以及**可选** Playwright 冒烟 job（`continue-on-error: true`）。

### Web UI E2E（Playwright）

冒烟测试在 [`web-e2e/`](./web-e2e/) — 页面加载、主题切换、Approvals tab。通过 Playwright `webServer` 启动真实 `unistar-coworker serve` 实例与最小化临时 `coworker.yaml`。

```bash
(cd web-ui && npm run build:fast)              # e2e 需要 dist/
cargo build --features embed-web-ui            # 二进制：target/debug/unistar-coworker
cd web-e2e
npm install
npx playwright install chromium                # 首次：下载 Chromium
npm test
```

可选：`UNISTAR_BIN=/path/to/unistar-coworker npm test` 指定二进制路径；`E2E_PORT=18787` 修改测试绑定端口。

### Feature flags

| Feature | 默认 | 用途 |
|---------|------|------|
| `web-browser` | on | `web_fetch` 浏览器模式用的 headless Chromium（引入 `chromiumoxide`）。`--no-default-features` 可得到更精简构建，`web_fetch` 回退纯 HTTP。 |
| `embed-web-ui` | off | 编译期将 `web-ui/dist/` 嵌入二进制（`include_str!`）。发布、CI、`./scripts/start-agent.sh` 启用；本地 `cargo check` / `cargo build` 省略以加速（UI 从磁盘 serve）。 |

vendored 的 `chromiumoxide` 补丁在 `vendor/chromiumoxide/`，用于应对 CDP schema 漂移。

Web UI（`web-ui/`）需要 Node 18+，用 `npm run build:fast` 构建（由开发者 / CI / `./scripts/start-agent.sh` 负责，**非** `build.rs`）。启用 `embed-web-ui` 时 bundle 编入二进制；未启用时 `serve` 运行时读 `web-ui/dist/`。

---

## 项目结构

```
unistar-coworker/
├── .cargo/config.toml       # 开发 profile：debug=1、增量；可选 sccache/mold
├── Cargo.toml               # Workspace 根
├── crates/
│   ├── core/                # config、store、llm、github、mcp、agent、engine、app
│   ├── tui/                 # ratatui 终端 UI
│   ├── web/                 # axum Web 服务 + embed-web-ui build.rs
│   ├── cli/                 # clap 子命令、终端输出、chat REPL
│   └── unistar-coworker/    # 薄二进制（`main.rs` → `coworker_cli::run`）
├── docs/RPC.md              # JSONL rpc 协议
├── packaging/
│   ├── README.md            # 打包说明
│   └── workdir-template/    # 部署种子（coworker.yaml），复制到运行时 workdir
├── scripts/
│   └── start-agent.sh       # 构建 web-ui + 二进制、刷新 workdir、启动
├── skills/                  # 技巧 skill（SKILL.md）+ _base/TOOLS.md SSOT
├── web-ui/                  # React 18 SPA（Vite + Tailwind + Radix + zustand）
│   ├── src/                 # TypeScript 源码
│   └── dist/                # vite build 产物（gitignore，生成）
├── vendor/chromiumoxide/    # 补丁后的 CDP 依赖
├── web-e2e/                 # Playwright 冒烟测试
├── coworker.example.yaml    # 配置模板
└── Cargo.lock
```

版本：**1.0.0**（workspace `[workspace.package]`，见 [Cargo.toml](./Cargo.toml)）。

---

## 贡献

请先阅读 [AGENTS.md](./AGENTS.md)：目录结构、harness 约定、敏感信息规则、PR 期望。Skill/Prompt 与 crate 同仓；工具名须与 `TOOLS.md`、`tool_catalog.rs` 保持一致。

约定：

- **最小 diff** — 匹配现有风格；复用 `tool_catalog`、`context`、`parse` 等 helper。
- **Rust 2021**，`tokio` 异步，`thiserror` / `anyhow` 错误处理。
- **测试** — 单测放在模块旁（`mod tests`）；用 `acme/widget` 与合成 JSON；完成后跑 `cargo test`。
- **仓库内不引入新密钥**；`coworker.yaml` 与 `data/` 已 gitignore。
- **Mutating 行为** 必须留在审批之后，除非配置显式关闭。
- 新增 chat 工具时，同步更新 `TOOLS.md`、`tool_catalog.rs` 与测试。

---

## 相关

- [docs/RPC.md](./docs/RPC.md) — JSONL `rpc` 模式，供脚本与集成使用。
- [docs/RPC.md](./docs/RPC.md) — JSONL `rpc` 模式，供脚本与集成使用。
- [unistar-mcp](../unistar-mcp) — 独立 GitHub MCP（Go）；可选，coworker 运行时不依赖。
- [README.md](./README.md) — English.

---

## 许可证

MIT — 见 [LICENSE](./LICENSE)。
