# unistar-coworker

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](./LICENSE)
[![Rust 1.75+](https://img.shields.io/badge/rust-1.75%2B-orange.svg)](https://www.rust-lang.org/)
[![CI](https://github.com/unistar-ai/unistar-coworker/actions/workflows/ci.yml/badge.svg)](./.github/workflows/ci.yml)

**面向本地 LLM 的通用 Agent（本地优先）** — 终端 TUI、浏览器 Web UI、原生工具调用、工作区工具、可选 MCP 联邦与进程内 GitHub harness。运行于 Ollama / OpenAI 兼容 API；可配置地将高风险 mutating 操作纳入 **人工审批**。

[English](./README.md) · [中文](./README_CN.md)

### 政策与支持

| 文档 | 说明 |
|------|------|
| [SECURITY.md](./SECURITY.md) | 漏洞报告、支持版本、本机暴露风险 |
| [PRIVACY.md](./PRIVACY.md) | 全本地数据、无遥测 |
| [CONTRIBUTING.md](./CONTRIBUTING.md) | Fork、分支、PR 与 CI |
| [SUPPORT.md](./SUPPORT.md) | GitHub Issues、自助文档 |
| [CHANGELOG.md](./CHANGELOG.md) | 版本历史 |

---

## 概览

`unistar-coworker` 是面向 **本地模型** 的 **通用 Agent 运行时**，不是托管 Agent 平台，也不是 CI runner。它：

- 运行 **chat**（TUI、CLI、Web），通过 skills + prompts 支持编码、问答与运维（`chat.workspace`）。
- 使用 **本地 LLM**（Ollama / OpenAI 兼容）做规划、工具调用与摘要；支持命名 profile 与运行时切换。
- 提供 **工作区工具**（`read_file`、`grep`、`bash_run` 等）；可选 **MCP**（`mcp.servers[]`）。
- 在需要时集成进程内 **GithubHarness**（`gh` CLI）；PR/CI triage 等运维能力是可选 skill pack，不是产品上限。
- **默认安全** — GitHub/MCP mutating 工具走 TUI/Web 审批，除非显式开启 `chat.auto_approve_mutations`。

---

## 目录

- [功能](#功能)
- [快速开始](#快速开始)
- [依赖](#依赖)
- [使用](#使用)
  - [TUI](#tui)
  - [Web UI](#web-ui)
  - [Chat](#chat)
  - [常用命令](#常用命令)
- [配置](#配置)
- [存储](#存储)
- [可选集成](#可选集成)
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
| **Chat** | TUI / CLI / Web 自然语言 Agent；LLM 编排工作区工具、可选 MCP 与 GitHub harness 的多步工具链 |
| **LLM** | 命名 `llm:` profile + 运行时切换（Web Config、RPC `switch_profile`、侧车 `coworker.llm-profile`）；面向 **25B+** 本地模型（如 qwen3.6-27B、gemma 26B A4B；64K–128K context） |
| **工作区** | `read_file`、`grep`、`glob`、`edit_file`、`bash_run`、`python_run` 等；mutating 路径经 LLM 安全审查 |
| **安全** | GitHub harness / MCP mutating 须经 TUI/Web 审批（除非 `chat.auto_approve_mutations` 或 per-server `approval.mutating: auto`） |
| **MCP 联邦** | `mcp.servers[]` 支持 stdio + HTTP、lazy 发现、mutating 审批、per-server skills、取消进行中的调用 |
| **GithubHarness** | 可选进程内 `gh` 调用 GitHub/CI；payload 有 cap |
| **TUI** | 聊天、审批、日志、配置、全屏聊天 |
| **Web UI** | `serve` 浏览器聊天、会话、明暗主题、带来源标签的流式工具/reasoning 卡片、审批弹窗、Markdown 导出 |
| **Store** | JSON（默认）或 SQLite：审批、聊天会话、审计日志；`store migrate` 与 `store compact` 命令 |

---

## 快速开始

> **分步安装：** [QUICKSTART_CN.md](./QUICKSTART_CN.md)（tar.gz + Docker）。

### Docker（三条命令）

```bash
docker pull ghcr.io/unistar-ai/unistar-coworker:latest
mkdir -p config data
docker run --rm -p 127.0.0.1:8787:8787 \
  -v "$(pwd)/config:/config" -v "$(pwd)/data:/data" \
  -e DEEPSEEK_API_KEY -e GH_TOKEN \
  ghcr.io/unistar-ai/unistar-coworker:latest serve --config /config/coworker.yaml
```

配置模板、卷挂载与 `gh auth` 说明见 [docs/docker.md](docs/docker.md)。

### 从源码

```bash
cd unistar-coworker
unistar-coworker init --llm-url http://localhost:11434/v1
# 或：cp coworker.minimal.yaml coworker.yaml 后编辑（推荐 25B+ 模型，如 gemma4:26b-a4b 或 qwen3.6:27b）

unistar-coworker doctor          # 检查 config / LLM / store（GitHub 可选）

# 前端：构建一次（开发从磁盘 serve；发布嵌入二进制）
(cd web-ui && npm install && npm run build:fast)

cargo build --release --features embed-web-ui

./target/release/unistar-coworker serve                        # Web → http://127.0.0.1:8787
./target/release/unistar-coworker                              # TUI

# 可选 GitHub：
export GH_TOKEN=ghp_...   # 或 gh auth login
./target/release/unistar-coworker chat --once "汇总 acme/widget 的 open PR" --json
./target/release/unistar-coworker chat --once "triage open PRs in acme/widget"
```

---

## 依赖

### 支持平台

| 平台 | 安装方式 | 说明 |
|------|----------|------|
| **Linux x86_64** | [tar.gz](https://github.com/unistar-ai/unistar-coworker/releases)、Docker（M2） | 官方 CI 构建 |
| **macOS arm64**（Apple Silicon） | [tar.gz](https://github.com/unistar-ai/unistar-coworker/releases) | 官方 CI 构建 |
| **其他**（Intel Mac、Linux arm64、Windows 等） | 源码：`cargo build --release --features embed-web-ui` | 社区自助编译，非官方支持 |

### 运行依赖

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

默认命令启动终端 UI。

```bash
cargo run --release
```

| 键 | 页 |
|----|-----|
| `0` / `?` | Chat |
| `1` | 审批（`y` / `n`） |
| `2` | 日志 |
| `3` | 配置（GitHub + `mcp[id]` 状态，`R` 重探测） |

`Tab` / `Shift+Tab` 切换标签 · `r` 刷新 store · `q` 退出 · `Esc` 取消当前 chat 轮次。

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

`build.rs` **不会**跑 `npm` — 前端构建由开发者、CI 或 [`scripts/package.sh`](./scripts/package.sh) 负责（`npm run build:fast`）。启用 `embed-web-ui` 时，manifest 按内容去重，只有 bundle 真变才触发重编译。未启用时需先 `npm run build:fast`，否则 React 路由返回 503。

**热重载**（无需重启进程）：向运行中的 `serve` / `tui` 发 `SIGHUP`，或 `POST /api/reload` — 重载 `coworker.yaml`、skills、prompts 与 MCP 连接。

**健康检查 API：** `GET /api/doctor` 与 `unistar-coworker doctor --json` 返回相同 JSON（config、`gh`、LLM、MCP、store）。

**开发期 HMR：**

```bash
# 终端 1：Rust 后端
cargo run -- serve

# 终端 2：Vite dev server（热重载，代理 /api 与 /ws 到 :8787）
cd web-ui && npm install && npm run dev
# 打开 http://localhost:5173
```

**安全模型（本机个人秘书）。** unistar-coworker **不是**多用户或公网产品。Web UI 仅面向**本机可信环境**：

- 保持 `web.bind` 为默认 **`127.0.0.1:8787`** — 聊天与审批仅监听回环地址。
- **Docker：** 仅映射到本机，例如 `-p 127.0.0.1:8787:8787`（无 `web.auth_token` 时切勿将容器端口暴露在 `0.0.0.0`）。
- **不要**在未加强鉴权的情况下将 Web UI 置于公网反向代理之后。

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

### 常用命令

| 命令 | 说明 |
|------|------|
| 默认 | TUI |
| `serve [--bind ADDR]` | Web UI + API + WebSocket |
| `chat [--once MSG] [--session UUID] [--list-sessions]` | 交互或单次聊天 |
| `report ci --repo owner/name [--since-days 7] [--json]` | CI 效率报告（需 GitHub harness + `--repo`） |
| `store migrate --from json --to sqlite --source DIR --dest FILE` | 迁移 store 后端 |
| `store compact [--audit-days 90] [--dry-run]` | 清理旧审计与遗留 store 文件 |
| `skills list` | 打印 skill 目录 |

### GitHub 工具

PR：`pr_list_open`、`pr_get_overview`、`pr_get_status`、`pr_get_diff`、`pr_list_changed_files`、`pr_diff_risk_scan`、`pr_create_backport` …

CI：`ci_analyze_pr_failures`、`ci_get_run_summary`、`ci_get_failed_logs`、`ci_rerun_workflow` …

Meta：`tool_search`、`tool_list`、`tool_describe`、`tool_call`、`resource_read`（`github://`、`pr://`、`ci://`）。

实现：[`crates/core/src/github/harness.rs`](./crates/core/src/github/harness.rs)。工具名 SSOT：[`skills/_base/TOOLS.md`](./skills/_base/TOOLS.md) + [`crates/core/src/agent/tool_catalog.rs`](./crates/core/src/agent/tool_catalog.rs)。

---

## 配置

从 [coworker.example.yaml](./coworker.example.yaml) 复制。加载路径：优先 `.coworker/coworker.yaml`，其次当前目录 `coworker.yaml`（均已 gitignore）。

```yaml
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
| `policy.auto_rerun_flaky` | 自动 rerun flaky CI（默认 `false`；否则需审批门控） |

---

## 存储

默认使用 `./data` 下的 JSON 后端（已 gitignore）。长期运行 `serve` 或多会话场景建议改用 **SQLite** — 单文件、并发读与长历史更稳：

```yaml
storage:
  backend: sqlite
  path: .coworker/data/coworker.db
```

从已有 JSON 迁移：

```bash
cargo run --release -- store migrate --from json --to sqlite \
  --source ./data --dest ./data/coworker.db
```

定期清理保持 store 紧凑：

```bash
cargo run --release -- store compact            # 默认：审计 90d + 清理遗留 store 文件
cargo run --release -- store compact --audit-days 180 --dry-run
```

---

## 可选集成

核心是 **本地优先通用 Agent**（工作区 + LLM）。以下为可选能力包，按需启用：

| 集成 | 配置 | 文档 |
|------|------|------|
| **GitHub / CI harness** | `github:`、GitHub ops skills | [skills/github-ops-pack/README.md](skills/github-ops-pack/README.md) |
| **第三方 MCP** | `mcp.servers[]`（Slack、HTTP、filesystem 等） | [docs/mcp-recipes.md](docs/mcp-recipes.md) |

编写 skill：[skills/_base/SKILL_TEMPLATE.md](skills/_base/SKILL_TEMPLATE.md)。本地模型：[docs/local-models.md](docs/local-models.md)。上下文预算：[docs/context-budget.md](docs/context-budget.md)。

---

## MCP 联邦

GitHub **永远**走进程内 `GithubHarness`。Slack、filesystem 等外部工具走 `mcp.servers[]`：

> 分步示例：[docs/mcp-recipes.md](docs/mcp-recipes.md)。

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
│  TUI / Web → Engine → Prompts + Skills → Store                    │
│                    ↓ LLM              ↓ Approvals                 │
│  GithubHarness (进程内 gh) + McpPool (可选 MCP)                   │
└──────────────────────────────────────────────────────────────────┘
```

| 入口 | 触发 | 编排 |
|------|------|------|
| **Chat** | TUI `[0]`、`chat`、Web | `prompts/chat.md` + skills + LLM 工具循环 |

### 产品边界

| 是 | 不是 |
|----|------|
| **本地优先的通用 Agent**（chat + 工具），面向本地 LLM | 云端托管 Agent 或多租户 SaaS |
| 工作区编码/问答、skills、MCP、可选 GitHub harness | 无审批自动 merge 或静默全库自主改写 |
| TUI + Web + RPC 脚本；终端友好 | GitHub Actions / CI runner 的替代品 |
| 默认审批门控的外部 mutating 工具 | 只能当 `gh` 套壳、无更广 Agent 能力 |

**刻意不做：** 无托管遥测；无审批自动 merge。GitHub 运维（PR/CI triage 等）是 **可选 skill pack**，不是唯一场景。

### Skill / Prompt / Harness

三层架构，职责不混用：

| 层 | 路径 | 职责 |
|----|------|------|
| **Skill** | `skills/*/SKILL.md` | 可复用技巧 — triage 规则、语气、流程说明。不含 harness 逻辑。 |
| **Prompt** | `prompts/chat.md` | Chat system prompt；frontmatter `skills:` 指定默认技巧。编译期内嵌。 |
| **Harness** | `crates/core/src/agent/`、`crates/core/src/engine/` | 确定性 Rust — MCP 池、审批、token 预算、chat 循环 |

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

**发布 / 部署**（单二进制、内嵌 UI — 与 CI、[`scripts/package.sh`](./scripts/package.sh) 一致）：

```bash
(cd web-ui && npm run build:fast)
cargo build --release --features embed-web-ui
```

### GitHub Releases

推送版本 tag 会触发 [`.github/workflows/release.yml`](./.github/workflows/release.yml)，自动构建并上传到 [GitHub Releases](https://github.com/unistar-ai/unistar-coworker/releases)：

```bash
git tag v1.0.0
git push origin v1.0.0
```

产物（按平台）：`unistar-coworker-<version>-<triple>.tar.gz` 及 `.sha256`，内含二进制、`skills/`、`template/`（workdir 种子）、`coworker.example.yaml`。平台：**Linux x86_64**、**macOS arm64**。

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
| `embed-web-ui` | off | 编译期将 `web-ui/dist/` 嵌入二进制（`include_str!`）。发布、CI、`./scripts/package.sh` 启用；本地 `cargo check` / `cargo build` 省略以加速（UI 从磁盘 serve）。 |

vendored 的 `chromiumoxide` 补丁在 `vendor/chromiumoxide/`，用于应对 CDP schema 漂移。

Web UI（`web-ui/`）需要 Node 18+，用 `npm run build:fast` 构建（由开发者 / CI / `./scripts/package.sh` 负责，**非** `build.rs`）。启用 `embed-web-ui` 时 bundle 编入二进制；未启用时 `serve` 运行时读 `web-ui/dist/`。

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
│   └── workdir-template/    # 部署种子（coworker.yaml），复制到运行时 .coworker/
├── scripts/
│   └── package.sh           # 构建 web-ui + 二进制、刷新 workdir（打包）
├── skills/                  # 技巧 skill（SKILL.md）+ _base/TOOLS.md SSOT
├── web-ui/                  # React 18 SPA（Vite + Tailwind + Radix + zustand）
│   ├── src/                 # TypeScript 源码
│   └── dist/                # vite build 产物（gitignore，生成）
├── vendor/chromiumoxide/    # 补丁后的 CDP 依赖
├── web-e2e/                 # Playwright 冒烟测试
├── coworker.example.yaml    # 配置模板（含可选 GitHub）
├── coworker.minimal.yaml    # 仅 workspace 模板
└── Cargo.lock
```

版本：**4.2.1**（workspace `[workspace.package]`，见 [Cargo.toml](./Cargo.toml)）。本地模型：[docs/local-models.md](./docs/local-models.md) · 上下文：[docs/context-budget.md](./docs/context-budget.md)。

---

## 获取帮助

见 [SUPPORT.md](./SUPPORT.md) — **仅 GitHub Issues**（bug / 功能 / 提问模板）。无商业 SLA。

| 资源 | 主题 |
|------|------|
| [docs/troubleshooting.md](./docs/troubleshooting.md) | 常见问题 |
| [docs/upgrading.md](./docs/upgrading.md) | 版本升级 |
| [docs/RPC.md](./docs/RPC.md) | JSONL 脚本接口 |

---

## 贡献

请先阅读 [CONTRIBUTING.md](./CONTRIBUTING.md) 与 [AGENTS.md](./AGENTS.md)：工作流、harness 约定、敏感信息规则、PR 期望。Skill/Prompt 与 crate 同仓；工具名须与 `TOOLS.md`、`tool_catalog.rs` 保持一致。

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
- [unistar-mcp](../unistar-mcp) — 独立 GitHub MCP（Go）；可选，coworker 运行时不依赖。
- [README.md](./README.md) — English.

---

## 许可证

MIT — 见 [LICENSE](./LICENSE)。安全：[SECURITY.md](./SECURITY.md) · 隐私：[PRIVACY.md](./PRIVACY.md)。
