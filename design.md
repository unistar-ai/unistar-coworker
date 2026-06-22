> 本地部署的 Gemma4:26B（64K context），基于 **unistar-mcp** 的 **unistar-coworker** —— 带 TUI 的 **GitHub 运维秘书** Harness。  
> v1 旗舰工作流是 **Daily Work**；同一套 Engine + Store + Agent/Skill 骨架可扩展更多工作流（见 [产品功能目录](#产品功能目录)）。

## 产品定位

| 是 | 不是 |
|----|------|
| 读 MCP cap 过的 GitHub/CI 信号，本地 LLM 辅助判断 | 替代 GitHub Actions / 自建 CI runner |
| 记账、digest、报表、代写草稿 | 无审批全自动 merge / 大范围改代码 |
| TUI 内审批 mutating 动作 | 云端 SaaS、Web Dashboard |
| 多工作流共用 Scheduler + Agent + Store | 一次性脚本合集 |
| **Chat 模式**：轻量 **coding agent**（workspace + 文件工具 + bash）；GitHub MCP 按需 lazy | 无约束全自动改 repo / 无审批 merge |

**定位一句话：GitHub 运维秘书（Workflow）+ 轻量 coding Chat（默认入口）** —— Workflow/cron 仍做 digest 与 ops 批处理；用户打开 Chat 即在本地 workspace 读/搜/改代码，MCP 退居二线按需发现。

## Skill / Agent / Harness 三层（v0.3+）

见 [skill-agent-harness.md](./skill-agent-harness.md) 全文。简要 glossary：

| 层 | 路径 | 职责 |
|----|------|------|
| **Skill** | `skills/*/SKILL.md`（技巧） | 领域判断、语气、反模式 — **可跨 agent 复用**；不含 cron、不含 JSON action schema |
| **Agent** | `agents/*/AGENT.md` | 单次任务目标、步骤、输出格式、tool 策略；可引用 `skills[]` |
| **Harness** | `src/agent/*.rs` | 确定性执行：MCP、Store、审批、budget、chat/workflow 循环 |

工具名 SSOT：`skills/_base/TOOLS.md` + `src/agent/tool_catalog.rs`。

---

## 设计前提：不要重复造轮子

原方案把 Harness 设计成「Webhook → 上下文压缩 → RAG → Map-Reduce → gh 回写」的 monolith。  
**unistar-mcp 已经解决了其中最难、最 GitHub 耦合的部分**——在 64K 模型上可用的、上下文节俭的工具层：

| 原方案模块 | unistar-mcp 现状 | 结论 |
|-----------|-----------------|------|
| Context Squeezer（diff / 日志脱水） | `gh`/`git` 只取必要 JSON 字段；CI 日志 error-extract + ~6KB cap；`pr_list_open` 默认 limit=20 | **复用，Harness 不再自己做 gh 封装** |
| 工具 schema 占满 context | `--lazy` 模式：仅暴露 `tool_list` / `tool_describe` / `tool_call` 三个 meta tool | **64K 模型默认开 lazy** |
| Action & Feedback | `ci_rerun_workflow`、`pr_create_backport` 等 mutating tools；无 shell、防 double-execute | **复用；Harness 负责审批 gate** |
| 工作流编排 | `pr-ci-triage` skill 已定义工具链顺序 | **Harness 加载 agent + 技巧 skills 组成 prompt** |
| AST 骨架 / 代码 RAG | **不做** | Daily Work / light-review 均不依赖源码索引 |
| Webhook 实时响应 | **未覆盖** | Daily Work 用 cron 轮询即可，Phase 2 再加 webhook |

**核心结论：** Harness 应是 **「调度器 + MCP Agent 循环 + Token 预算管理」**，而不是第二个 GitHub 中间件。  
上下文压缩的主战场在 **unistar-mcp 工具返回值** 和 **Agent 多轮按需拉取**，而非一次性塞满 diff。

---

## GitHub Daily Work（旗舰工作流 · v1）

工作流 ID：`daily-work`。基于 unistar-mcp 现有能力，聚焦 **可每日 cron 跑完、输出 digest 的任务**：

### 晨间 triage（只读，可全自动）

1. **`pr_list_open`** — 列出各 repo 的 open PR（含 CI / review 一行摘要）
2. 对 **CI 失败** 的 PR 链式调用：`pr_get_status` → `ci_analyze_pr_failures` → `ci_get_failed_logs`
3. 模型判断 flaky vs real bug，生成 **Daily Digest**（Markdown）：  
   - 需人工关注的 PR（real failure、review blocked、merge conflict）  
   - 可忽略的（draft、waiting approval、全绿）  
   - flaky 候选（待审批 rerun）→ **写入 Store `flaky_*` 表**，供历史统计与报表

### 例行维护（mutating，需 Human-in-the-loop 或白名单）

4. **`ci_rerun_workflow`** — 仅对 **ci-triage** 判定为 flaky 且用户在 config 中开启 `auto_rerun_flaky: true` 的 run 执行  
5. **`pr_create_backport`** — 对「已 merge 且 label 含 `needs-backport`」的 PR 按 release 分支规则创建 backport（或仅生成建议列表）

### v1 明确不做（其他工作流后续迭代）

- 行级 Code Review / 大 diff 摘要 → 工作流 `light-review`（Phase 3）  
- Issue 自动回复 / 打标 → 工作流 `issue-triage`（Phase 2）  
- 全库 RAG → **不做**；`light-review` 仅用 `pr_get_diff` per-file Map-Reduce

---

## 产品功能目录

所有功能共享同一 **Workflow 执行模型**，差异只在 **Agent 规格**、技巧 skills、调度、cron 与 Store 写入项：

```
cron / 手动 / Webhook(Phase 2) / Chat(Phase 2+)
    → Scheduler 规则过滤（零 LLM）          ← Chat 入口跳过 Scheduler，直接进 Agent
    → 任务队列 (workflow_id, repo, subject, …)
    → Agent Loop（加载对应 agent + skills + token 预算）
    → MCP 按需拉取 cap 信号
    → 本地 LLM 判断 / 摘要
    → Store 持久化
    → TUI 展示 + 可选 mutating 审批
```

**两种入口：**

| 入口 | 触发 | 编排 | 典型用途 |
|------|------|------|----------|
| **Workflow** | cron、TUI 快捷键、`run-once` | 固定 **Agent** + Harness 确定性步骤 | Daily digest、release-duty、main-guard |
| **Chat** | TUI Chat 视图、CLI `chat` | `agents/chat` + 技巧 skills + **LLM 自主选工具** | 「#142 CI 为啥红？」「帮我列下等 review 的 PR」「跑一遍 review-radar 并总结」 |

Workflow 负责 **可重复、可审计的批处理**；Chat 负责 **adhoc 问答与轻量指挥**，二者共用 Engine、MCP、Store、审批 gate。

### Workflow 配置约定

```yaml
# coworker.yaml 片段
workflows:
  daily-work:
    enabled: true
    agent: agents/daily-work/AGENT.md
    skills:
      - skills/ci-triage/SKILL.md
      - skills/digest-style/SKILL.md
    schedule: "0 6 * * *"        # 可覆盖全局 schedule
    mutating: [rerun_flaky, backport]  # 需 TUI 审批
  release-duty:
    enabled: true
    agent: agents/release-duty/AGENT.md
    schedule: "0 9 * * 1-5"      # 工作日 9:00
    mutating: [backport]
```

每个工作流对应 **`agents/<id>/AGENT.md`**（任务 SSOT），可选引用 **`skills/*`**（技巧 SSOT）；Engine 按 `workflow_id` 加载 agent 并 compose prompt。

### Tier A — 现有 MCP 工具即可（优先落地）

仅 Harness + Agent 规格 + Store；**不强制**扩展 unistar-mcp。

| ID | 名称 | 做什么 | 调度 | Mutating | Store / TUI |
|----|------|--------|------|----------|-------------|
| **`daily-work`** | Daily Work | PR 列表 → CI triage → Daily Digest；flaky 落库 | 每日 6:00 + 每 4h 扫描 | rerun、backport（审批） | `digests`、`flaky_*`；Dashboard |
| **`release-duty`** | Release / Backport 值班 | 扫描 `needs-backport`、已 merge 未 backport；生成 backport 建议队列 | 发版日 / 工作日 cron | `pr_create_backport`（批量审批） | `backport_queue`；审批队列 |
| **`main-guard`** | Main 红线守护 | 盯 default 分支最近 N 次 workflow；全红时 **立即** digest（不等 Daily） | 每 15–30min | 无 | `main_alerts`；Dashboard 高亮 |
| **`my-pr-brief`** | 我的 PR 简报 | `pr_list_open author=@me` + 状态：等 review / CI 红 / 可 merge | 每日或手动 | 无 | 写入当日 digest 一节；Dashboard |
| **`review-radar`** | Review 阻塞雷达 | 筛 CI 已绿但 `reviewDecision=REVIEW_REQUIRED`、mergeable 的 PR | 每日 | 无 | `pr_snapshots` 标签；PR 列表筛选 |
| **`flaky-govern`** | Flaky 治理 | 基于 `flaky_tests` rollup：Top-N、quarantine 建议、rerun 有效率 | 每周一 | 无（建议名单） | 扩展 Flaky 视图；`report flaky` |
| **`oncall-handoff`** | On-call 交接包 | 合成：最新 digest + 未决 approval + 24h flaky incident + main 告警 | 换班前手动 / cron | 无 | 读 Store 聚合导出 Markdown |

**Tier A 优先级（产品）：** `daily-work` → `release-duty` → `main-guard` → `review-radar` → `flaky-govern` → 其余。

### Chat 模式（交互层 · 轻量 coding agent）

**不是新 cron 工作流**，而是与 Workflow **并列的交互入口**：用户在 TUI 或 CLI 里用自然语言驱动本地 **workspace** 上的读/搜/改/跑命令，LLM 在 **file tools + bash_run** 上做多轮 **plan → tool_call → 观察 → 回复**，直到答完或触达预算。GitHub MCP 工具 **不预热**；用户提到 PR/CI 时走 `tool_search` → `tool_call`，或 `bash_run gh ...` 兜底。

#### 目标场景

| 用户说 | Agent 行为（示例） |
|--------|-------------------|
| 「这个 panic 在哪？」 | `grep` → `read_file` → 定位 → 小步 `edit_file` |
| 「跑一下 cargo test」 | `bash_run cargo test` → 读失败 → 定位修复 |
| 「find where Foo is defined」 | `glob` / `grep` → `read_file` 相关行 |
| 「refactor rename X to Y」 | `grep` 引用 → 多次小 `edit_file` → `bash_run` 验证 |
| 「git status / commit」 | `bash_run git …`（不 force push main） |
| 「#19188 CI 为啥红？」（可选） | lazy MCP：`tool_search` → `pr_get_overview` / `ci_analyze_pr_failures` |
| 「今天 digest 说啥？」（可选） | lazy MCP 或 `skill_load` ops skill → `store_get_latest_digest` |

#### 与 Workflow 的区别

| | Workflow | Chat |
|---|----------|------|
| 任务规格 | `agents/<workflow-id>/AGENT.md` | `agents/chat/AGENT.md` + **coding skills** |
| 技巧 | ops skills（ci-triage、pr-merge…） | `code-edit`、`repo-explore`、`debug`、`test-run`、`git-workflow` |
| 工具链 | Harness 写死的 MCP 顺序 | LLM 选 **read_file / grep / glob / edit_file / bash_run**；MCP lazy |
| 上下文 | repos + Store digest | **workspace 路径 + git 一行摘要 + session recent edits** |
| 输出 | Digest、Store 实体、报表 | 对话消息 + 可选 Approval |
| 调度 | cron | 仅用户发起 |
| 审计 | `workflow_runs` | `chat_sessions` + `transcripts` |

#### 架构（复用 Agent Loop）

```
用户输入 (TUI Chat / CLI chat)
    → ChatSession（session_id, 最近 N 轮历史）
    → System: compose(agents/chat + coding skills[]) + workspace runtime context
    → LLM 循环（max_turns、max_tool_calls 来自 policy）
         ├─ 只读 tool（read/grep/glob）→ 直接执行，结果 cap 后回填
         └─ approval-required tool（GitHub mutating）→ push Approval
         └─ no-approval path（bash_run/python_run/edit_file/write_file/reads）→ LLM safety review，可并行
    → assistant 消息 → TUI / stdout
```

**64K 约束（Chat 专用）：**

- 会话历史 **滑动窗口** + **coding compaction**（保留 path:line、错误原文、recent edits）  
- 长 `read_file` / `grep` 输出压缩；旧 bash stdout 留 exit code + 末 20 行  
- 复杂 **ops** 任务仍应 **用户触发 workflow**（daily-work 等）；Chat 默认 coding，不在对话里 fork 其他 agent

#### Agent 要点（`agents/chat/AGENT.md` + coding skills）

- 身份：**轻量 coding assistant**，默认在 `chat.workspace` 内操作  
- 工具：warm `read_file`、`grep`、`glob`、`edit_file`、`bash_run`；MCP 仅按需发现  
- Mutating：`edit_file` / `write_file` / `bash_run` / `python_run` 走 **LLM safety review**（无人工审批）；GitHub mutating 每轮最多一个审批
- 不臆造文件内容；只报告 tool 输出  
- MCP 离线时 Chat 仍可用；GitHub 问题可用 `bash_run gh` 或等 MCP 恢复

#### Chat 默认工具面

| Tool | 层 | 用途 |
|------|-----|------|
| `read_file` | 原生 | 按 path + 行范围读 |
| `grep` | 原生 | ripgrep 搜内容 |
| `glob` | 原生 | 找文件 |
| `edit_file` / `write_file` | 原生 | 改代码（LLM review） |
| `bash_run` | 原生 | test / build / git（LLM review） |
| `python_run` | 原生 | 短 Python 片段（LLM review） |
| MCP PR/CI 工具 | lazy | 用户问 GitHub 时再 `tool_search` |

#### TUI / CLI

| 入口 | Phase | 行为 |
|------|-------|------|
| TUI **`[0] Chat`** 或 **`?` 切 Chat 全屏** | 2+ | 输入框 + 滚动对话；侧栏显示 tool 调用摘要（非 raw MCP） |
| `unistar-coworker chat` | 2+ | 无 TUI 的 REPL；适合 SSH |
| `unistar-coworker chat --once "…"` | 2+ | 单轮命令，执行完退出（脚本友好） |

Chat 视图中 mutating 意图与 Approvals tab **联动**：模型提议 rerun 时，Approvals 出现 pending 项，用户在 `[3]` 按 `y`/`n`。

#### Store

| 实体 | 用途 |
|------|------|
| `chat_sessions(id, created_at, title, repo_scope)` | 会话元数据 |
| `chat_messages(session_id, role, content, ts, tool_calls_json)` | 用户/助手/工具摘要 |
| `transcripts` | 与 workflow triage 共用格式，便于 Playbook few-shot（Phase 3） |

#### 配置 sketch

```yaml
chat:
  workspace: .              # 默认 cwd，file tools 沙箱根
  # bash: { timeout_secs: 30 }  # 可选；bash_run 始终可用，mutating 走审批
  agent: agents/chat/AGENT.md
  max_turns: 8
  max_tool_calls: 6
  history_messages: 12
  # mutating 永远走 approval，无 auto 开关
```

#### 产品边界（Chat 仍不做）

- 不是全库 RAG / AST 索引；靠 `grep` + 按需 `read_file`  
- 不支持多用户/权限模型；单操作者本地助手  
- 不做「自动 merge / 无审批大范围改 repo」  
- 长 **ops** 任务（完整 daily-work）优先 **TUI/CLI 一键 workflow**；Chat 默认 coding，ops 走 lazy MCP 或 workflow

**Phase：** Chat MVP 放在 **Phase 2 末 / Phase 3 初**（Tier A workflow 与审批链路稳定后）；TUI Chat 视图可与 CLI `chat` 分期交付（CLI 先行验证 Agent 循环）。

### Tier B — 需扩展 unistar-mcp tool（Harness 模式不变）

新 tool 进 Go registry（cap + lazy）；Coworker 加 **Agent 规格** + Store 表。

| ID | 名称 | 依赖新 tool | 做什么 |
|----|------|-------------|--------|
| **`issue-triage`** | Issue 分诊 | `issue_list_open`、`issue_get`；可选 `issue_add_label` | 未处理 issue 分类、重复聚类（标题相似度 Harness 侧）、优先级 digest |
| **`pr-hygiene`** | PR 卫生 | `pr_list_stale`、`pr_list_changed_files` | N 天无更新提醒；docs-only 过滤；超大 PR 警告 |
| **`security-digest`** | 安全告警简报 | `alert_list_open`（Dependabot/code scanning） | Critical/High 摘要 + LLM 一句影响面 |
| **`release-notes`** | Release notes 草稿 | `pr_list_merged --since` | Map-Reduce 生成 changelog 草案，人改后贴 Release |
| **`ci-efficiency`** | CI 效率报表 | `ci_list_runs`（duration、conclusion 一行） | 最慢 workflow、失败率趋势（Store 聚合，类 flaky） |
| **`comment-assist`** | Comment 助手 | `ci_get_run_summary` + `pr_post_comment` | CI 失败时生成 comment 草稿 → 审批后发送 |
| **`merge-health`** | Merge 队列健康 | `pr_get_merge_blockers` | 「CI 已过但仍 blocked」及结构化 blocker 清单 |
| **`review-radar`** | Review 雷达 | `pr_list_waiting_review` | CI-green 且 review-required 的 PR |

### Tier C — 较重 LLM / Map-Reduce（Phase 3）

| ID | 名称 | 做法 | 约束 |
|----|------|------|------|
| **`light-review`** | 轻量 Code Review | `pr_get_diff` 分文件 batch → 风险点清单（非行级 comment） | 必须 per-file Map-Reduce |
| **`regression-link`** | 回归归因 | 新 flaky 测试 + 近期 merge PR 列表 → LLM 排序「可能相关」 | 结论进 Store，仅供参考 |
| **`breaking-sniff`** | Breaking change 嗅探 | diff 路径规则（API/migration）+ LLM 摘要 | 规则优先，LLM 补充 |

### 平台能力（Phase 3+）

| 能力 | 说明 |
|------|------|
| **多 MCP Server** | 除 unistar-mcp 外挂载 `slack-mcp` 等；TUI 按数据源分 tab；GitHub 仍走 unistar-mcp |
| **Agent / Skill 目录** | `agents/<id>/AGENT.md` 任务 SSOT；`skills/*` 可复用技巧；见 [skill-agent-harness.md](./skill-agent-harness.md) |
| **Playbook 回放** | Store 存成功 triage 的 transcript → 作 few-shot 提升弱模型 flaky 判准率 |
| **Chat 模式** | 自然语言 adhoc 指挥；与 Workflow 共用 MCP + 审批；见 [Chat 模式](#chat-模式交互层--phase-2) |
| **规则引擎** | YAML：`if workflow=test-integration and error~timeout then suggest_rerun`；LLM 只处理规则未覆盖项 |
| **通知副本** | headless `run-once --workflow …` + Slack/webhook；**TUI 仍是主界面** |

### 产品边界（刻意不做）

| 不做 | 原因 |
|------|------|
| 无审批自动 merge、自动 push 大范围修复 | 64K 弱模型 + 安全；mutating 必须 rare |
| 全库语义 RAG、AST 全量索引 | 与运维秘书定位无关；成本高 |
| Webhook 触发后立即跑 LLM（无队列） | Gemma 推理慢；应规则过滤 → 入队 → 逐个处理 |
| 替代 CI runner / 自建 GitHub Actions | Harness 只**读** CI 结果 |
| 实时 Web Dashboard | TUI 是唯一 GUI |

### Store 扩展（随工作流增量）

| 实体 | 工作流 | Phase |
|-----|--------|-------|
| `backport_queue` | release-duty | 2 |
| `main_alerts` | main-guard | 2 |
| `issue_snapshots` | issue-triage | 2 |
| `ci_run_stats` | ci-efficiency | 2 |
| `workflow_runs` | 全部 | 1（泛化 scheduler_runs：含 workflow_id） |
| `chat_sessions` / `chat_messages` | chat | 2+ |

`workflow_runs(id, workflow_id, job, started_at, finished_at, error, summary_json)` 替代仅记录 cron 名的 `scheduler_runs`，便于 TUI 按工作流筛历史。

### TUI 与工作流映射

| 视图 | 关联工作流 | 说明 |
|-----|-----------|------|
| Dashboard `[1]` | daily-work、main-guard、my-pr-brief | 多 digest 区块；main 告警顶栏 |
| PR 列表 `[2]` | 全部 | `review-radar` 筛选预设 |
| 审批队列 `[3]` | release-duty、daily-work | backport / rerun / comment |
| Flaky `[6]` | flaky-govern、daily-work | 报表 + quarantine 建议 |
| **Release `[7]`** | release-duty | Phase 2：backport 队列专用 tab |
| **Issues `[8]`** | issue-triage | Phase 2 |
| **Chat `[0]` / `?`** | chat | Phase 2+：对话 + tool 轨迹；mutating 跳转 Approvals |

Phase 1 保持 6 个 tab；Phase 2 起 `[7][8]` 随工作流启用出现；**Chat 为叠加视图**（可全屏，不强制占用数字 tab 序列）。

---

## 架构：TUI + 引擎 + MCP

**TUI 是唯一用户界面**——配置、调度、digest 浏览、PR triage、mutating 审批、Agent 实时日志，全部在终端内完成；不提供 Web Dashboard。

```
┌──────────────────────────────────────────────────────────────────┐
│  unistar-coworker TUI (ratatui)          ← 统一用户界面           │
│  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────────────┐     │
│  │ Chat     │ │ Dashboard│ │ PR 列表  │ │ … Approvals / Logs │   │
│  └────┬─────┘ └────┬─────┘ └────┬─────┘ └────────┬─────────┘     │
│       └────────────┴────────────┴────────────────┘               │
│                         │ mpsc / broadcast (AppEvent)            │
├─────────────────────────┼────────────────────────────────────────┤
│  Core Engine (tokio)    ▼                                        │
│  ┌─────────────┐  ┌──────────────┐  ┌────────────────┐           │
│  │ Scheduler   │  │ Agent Loop   │  │ Token Budget   │           │
│  │ (cron/      │→ │ (local LLM   │←→│ (64K 硬预算)    │           │
│  │  manual)    │  │  + agents/   │  │                │           │
│  │             │  │    skills)   │  │                │           │
│  └──────┬──────┘  └──────┬───────┘  └────────────────┘           │
│         │                │                                        │
│         └────────┬───────┘                                        │
│                  ▼                                                │
│         ┌─────────────────┐                                       │
│         │ Store (trait)   │  json/ 或 sqlite/（coworker.yaml 配置） │
│         └─────────────────┘                                       │
│                          │ MCP (stdio / HTTP)                     │
└──────────────────────────┼─────────────────────────────────────────┘
                           ▼
┌──────────────────────────────────────────────────────────────────┐
│  unistar-mcp (--lazy)                                            │
│  pr_* · ci_* · backport · [future: issue_*, pr_diff]             │
└──────────────────────────┬───────────────────────────────────────┘
                           ▼
                      gh / git
```

**进程模型：** 默认 `unistar-coworker` 启动 **TUI + 内嵌 Engine**（同进程，Engine 跑在 tokio runtime 后台 task）。  
可选 `unistar-coworker daemon` 无头模式（systemd / cron 服务器）；TUI 通过读 **同一 Store** attach（Phase 2；SQLite 或 JSON + 文件锁）。

TUI 与 Engine 通过 **`AppEvent` 广播 + `AppState` 快照** 解耦：Engine 推送事件（`DigestReady`、`PrTriageDone`、`ApprovalNeeded`、`ToolCall`、`LogLine`），TUI 每帧只读 state 渲染，不阻塞 Agent。  
**持久化状态一律经 Store 读写**，TUI 启动时从 Store  hydrate，Engine 写入后广播 `StoreUpdated`。

### 4. 本地存储（JSON 或 SQLite，可配置）

**不使用远程数据库。** 所有业务数据落在本机，通过 `coworker.yaml` 的 `storage.backend` 在两种后端间切换：

| 后端 | 适用 | 形态 |
|-----|------|------|
| **`json`**（默认） | 个人单机、希望直接 `cat`/git 调试数据 | 目录 + 若干 `.json` 文件 |
| **`sqlite`** | 日志/历史较多、需按条件查询与分页 | 单文件 `coworker.db` |

Engine 与 TUI 只依赖 **`Store` trait**，不直接碰文件/SQL；两种实现语义一致。

#### 存储内容

| 实体 | 用途 | TUI 视图 |
|-----|------|---------|
| `digests` | 每日 digest 元数据 + 正文（Markdown） | Dashboard |
| `pr_snapshots` | `pr_list_open` / triage 结果缓存 | PR 列表 |
| `approvals` | 待批 / 已批 / 已拒 mutating 动作 | 审批队列 |
| `transcripts` | 单 PR Agent 会话（tool call + LLM 摘要） | 详情 pane |
| `audit_log` | 结构化审计（谁、何时、何种 tool、结果） | Logs |
| `scheduler_runs` | cron 执行记录（开始/结束/错误） | Config 状态栏 |
| **`workflow_runs`** | 泛化执行记录（含 `workflow_id`） | Config / Logs 按工作流筛选 |
| **`backport_queue`** | Release 值班 backport 建议与状态 | Release 视图 `[7]` |
| **`main_alerts`** | default 分支 CI 红线事件 | Dashboard 顶栏 |
| **`flaky_incidents`** | 每次 flaky 判定的原始事件（一次 triage 一条） | Flaky 视图 / 报表 |
| **`flaky_tests`** | 按测试指纹聚合的 rollup（次数、最近出现、rerun 成功率） | Flaky 视图 / 报表 |

`coworker.yaml` 本身 **不进 Store**（静态配置，用 `$EDITOR` 改）；运行时派生状态（LLM 连通性等）仅内存 + 可选写入 `scheduler_runs`。

#### JSON 后端布局

```
~/.config/unistar-coworker/data/          # storage.path
├── meta.json                             # schema 版本、最后 compaction 时间
├── digests/
│   └── 2026-06-12.json
├── pr_snapshots/
│   └── owner__repo-a.json                # repo 中 / 替换为 __
├── approvals/
│   └── pending.json
│   └── history.jsonl                     # 追加式，便于 tail
├── transcripts/
│   └── owner__repo-a__142.json
├── flaky/
│   ├── incidents.jsonl
│   └── tests.json
├── backport_queue.json
├── main_alerts.jsonl
└── audit/
    └── 2026-06.jsonl                     # 按月分片
```

- 读写：`serde_json` + 原子写（写 `*.tmp` 再 `rename`）  
- 并发：`fs2` / `fd-lock` 文件锁；`daemon` + TUI 同目录时需锁  
- 优点：人类可读、易备份、易手工清理  

#### SQLite 后端布局

```
~/.config/unistar-coworker/coworker.db    # storage.path
```

建议表（v1）：

```sql
-- digests(id, date, summary_json, body_md, created_at)
-- pr_snapshots(repo, pr_number, snapshot_json, fetched_at, triage_json, triage_at)
-- approvals(id, kind, payload_json, status, decided_at, ...)
-- transcripts(id, repo, pr_number, turns_json, created_at)
-- audit_log(id, ts, level, event, payload_json)
-- scheduler_runs(id, job, started_at, finished_at, error)  # 保留兼容
-- workflow_runs(id, workflow_id, started_at, finished_at, error, summary_json)
-- backport_queue(id, repo, pr_number, target_branch, status, created_at)
-- main_alerts(id, repo, ts, run_id, conclusion, acknowledged)
-- flaky_incidents(id, ts, repo, pr_number, run_id, workflow, job, step,
--                 test_name, fingerprint, classification, log_excerpt,
--                 llm_reason, approval_id, rerun_outcome)
-- flaky_tests(fingerprint PK, repo, workflow, job, test_name,
--              first_seen, last_seen, incident_count,
--              rerun_attempts, rerun_successes, last_error_signature)
```

`flaky_tests` 在每次 `record_flaky_incident` 时 **同步 upsert**（JSON 后端重写 `tests.json`；SQLite 一条 `INSERT ... ON CONFLICT`），报表只查 rollup 表即可，不必扫全量 incidents。

- 库：`rusqlite`（embedded，无 async 驱动依赖）+ `tokio::task::spawn_blocking`  
- 迁移：内嵌 SQL migration（启动时 `PRAGMA user_version`）  
- 优点：PR 列表过滤、Logs 分页、**flaky Top-N / 按 repo 分组** 一条 SQL 搞定  

#### Store trait（ sketch ）

```rust
#[async_trait]
trait Store: Send + Sync {
    async fn save_digest(&self, digest: &Digest) -> Result<()>;
    async fn latest_digest(&self) -> Result<Option<Digest>>;
    async fn list_digests(&self, limit: usize) -> Result<Vec<DigestMeta>>;

    async fn upsert_pr_snapshot(&self, snap: &PrSnapshot) -> Result<()>;
    async fn list_pr_snapshots(&self, repo: Option<&str>) -> Result<Vec<PrSnapshot>>;

    async fn push_approval(&self, item: &Approval) -> Result<()>;
    async fn decide_approval(&self, id: &str, decision: Decision) -> Result<()>;
    async fn list_pending_approvals(&self) -> Result<Vec<Approval>>;

    async fn append_audit(&self, entry: &AuditEntry) -> Result<()>;
    async fn query_audit(&self, q: AuditQuery) -> Result<Vec<AuditEntry>>;

    async fn save_transcript(&self, t: &Transcript) -> Result<()>;

    async fn record_flaky_incident(&self, incident: &FlakyIncident) -> Result<()>;
    async fn update_flaky_rerun(&self, incident_id: &Uuid, outcome: RerunOutcome) -> Result<()>;
    async fn reclassify_flaky(&self, incident_id: &Uuid, classification: Classification) -> Result<()>;
    async fn list_flaky_tests(&self, q: FlakyQuery) -> Result<Vec<FlakyTestRollup>>;
    async fn list_flaky_incidents(&self, q: FlakyIncidentQuery) -> Result<Vec<FlakyIncident>>;
}
```

JSON / SQLite 各一个 impl；单元测试对两种 backend 跑同一套 conformance tests。

#### 选型建议

| 场景 | 推荐 |
|-----|------|
| 首次试用、数据量小 | `json` |
| 审计日志 > 几万条、Logs 视图常开 | `sqlite` |
| `daemon` + 多 TUI attach | `sqlite`（WAL 模式并发更稳） |

切换 backend：**导出/导入 CLI**（Phase 2：`unistar-coworker store migrate --from json --to sqlite`），不做运行时热切换。

#### Flaky Test 记录与报表

Daily Work 的 flaky 判定不应只出现在当日 digest 里——**每次 triage 都要落库**，积累后可做趋势报表（哪条测试最 flaky、哪个 workflow 最不稳定、rerun 是否真的有效）。

##### 何时写入

| 时机 | `classification` | 说明 |
|-----|------------------|------|
| Agent 判定 flaky | `llm_flaky` | triage 流程中 LLM 读完 `ci_get_failed_logs` 后 |
| Agent 判定 real bug | `llm_real` | 可选记录，用于误报率统计；默认开启 |
| 用户在 TUI 改判 | `user_flaky` / `user_real` | Phase 2：详情 pane 快捷键纠正 |
| rerun 完成后 | 更新 `rerun_outcome` | `succeeded` → rollup `rerun_successes++`；仍失败则记新 incident |

写入由 **Engine 在 triage 管线末尾** 调用 `Store::record_flaky_incident`，与 approval / audit 同事务（SQLite）或同锁（JSON）。

##### 字段与指纹（fingerprint）

从 MCP 日志摘要中尽量结构化提取（Harness 侧轻量 regex，解析失败则留空）：

```rust
struct FlakyIncident {
    id: Uuid,
    ts: DateTime<Utc>,
    repo: String,
    pr_number: Option<u32>,
    run_id: i64,
    workflow: String,       // ci_analyze 返回的 workflow name
    job: Option<String>,
    step: Option<String>,
    test_name: Option<String>,   // 如 "TestFoo::bar" / pytest nodeid
    fingerprint: String,         // sha256(repo|workflow|job|normalize(test_name|error_sig))
    classification: Classification,
    log_excerpt: String,         // cap ~2KB，来自 ci_get_failed_logs
    llm_reason: Option<String>,  // 模型一句话理由
    approval_id: Option<String>,
    rerun_outcome: Option<RerunOutcome>,
}

struct FlakyTestRollup {
    fingerprint: String,
    repo: String,
    workflow: String,
    job: Option<String>,
    test_name: Option<String>,
    first_seen: DateTime<Utc>,
    last_seen: DateTime<Utc>,
    incident_count: u32,
    rerun_attempts: u32,
    rerun_successes: u32,
    last_error_signature: String,  // 归一化 error 行，便于 diff 同类失败
}
```

**fingerprint** 用于聚合：同一测试在不同 PR/run 重复失败 → `flaky_tests.incident_count` 递增，而非无限追加孤立行。`test_name` 缺失时退化为 `workflow + job + error_signature` 哈希。

##### Store API（扩展）

```rust
async fn record_flaky_incident(&self, incident: &FlakyIncident) -> Result<()>;
async fn update_flaky_rerun(&self, incident_id: &Uuid, outcome: RerunOutcome) -> Result<()>;
async fn reclassify_flaky(&self, incident_id: &Uuid, classification: Classification) -> Result<()>;

async fn list_flaky_tests(&self, q: FlakyQuery) -> Result<Vec<FlakyTestRollup>>;
async fn list_flaky_incidents(&self, q: FlakyIncidentQuery) -> Result<Vec<FlakyIncident>>;
// FlakyQuery: repo, since, limit, order_by_occurrence
```

##### 报表（Phase 2，数据 Phase 1 起攒）

| 输出 | 命令 / UI | 内容 |
|-----|-----------|------|
| TUI **Flaky** 视图 | `[6]` | Top-N flaky 测试；按 repo/workflow 筛选；最近 7/30 天 |
| CLI 导出 | `unistar-coworker report flaky --since 30d --format csv` | 给 spreadsheet / BI |
| Digest 附录 | 可选 | Daily Digest 尾部「本周 flaky Top 5」 |

典型 SQL（SQLite）：

```sql
-- 近 30 天最常 flaky 的测试
SELECT test_name, workflow, incident_count, rerun_successes * 1.0 / NULLIF(rerun_attempts, 0) AS rerun_rate
FROM flaky_tests
WHERE last_seen >= datetime('now', '-30 days')
ORDER BY incident_count DESC
LIMIT 20;
```

JSON 后端无 SQL 时，由 `list_flaky_tests` 在内存排序截断；数据量上万条后建议切 SQLite。

### 1. Scheduler（事件 → 任务，轻量优先）

**v1 用 cron 轮询，不用 Webhook。**

| 方式 | 适用 | 理由 |
|-----|------|------|
| Cron（推荐 v1） | Daily Digest 6:00、每 4h CI 扫描 | 零 infra；Gemma 推理慢，不适合秒级 webhook |
| Webhook（Phase 2） | `pull_request` synchronized 触发单 PR triage | 需公网入口 + 队列，等 Agent 稳定后再加 |

Scheduler 只做 **规则过滤（零 LLM）**：

- 跳过：draft PR、仅 docs/README 变更（可通过 `gh pr view --json files` 在 mcp 新 tool 里做）  
- 标记：`action_required` 不算 failure（**ci-triage** skill 已说明）  
- 输出：`(workflow_id, repo, pr_number, task_type)` 任务队列

### 2. Agent Loop（大脑，薄编排）

Harness 本身 **不解析 diff、不调用 gh**，只：

1. 连接本地推理端点（Ollama / vLLM / llama.cpp），Gemma4:26B @ 64K  
2. 连接 `unistar-mcp`（推荐 `unistar-mcp --lazy` stdio 子进程，或 `http :8080` 多 worker 共享）  
3. 按任务 **`workflow_id`** 加载对应 **agent**（如 `agents/daily-work/AGENT.md`）+ 可选 `skills[]` + token 预算  
4. 运行受限 MCP Agent 循环（max turns、max tool calls、timeout）  
5. 汇总 transcript → **写入 Store** → 推送到 TUI → 可选导出 Markdown / Slack

**Chat 模式**复用同一 Agent Loop 实现，差异在于：无预定义 subject 队列、每轮由 LLM 决定下一 tool；加载 `agents/chat/AGENT.md` + 技巧 skills，而非 workflow agent。详见 [Chat 模式](#chat-模式交互层--phase-2)。

### TUI：统一用户界面

基于 **[ratatui](https://ratatui.rs/)** + **crossterm**，采用 **Elm 式** `Model / Msg / Update`（或等价状态机），与 Engine 异步 task 通过 channel 通信。

#### 屏幕布局（默认三栏）

```
┌─ unistar-coworker ──────────────────────────────── q quit ─┐
│ [1 Dashboard] [2 PRs] [3 Approvals] [4 Logs] [5 Config] [6 Flaky] │
├──────────────┬─────────────────────────┬───────────────────┤
│ Repo / 筛选  │  PR 列表 / Digest 正文    │ 详情 / Transcript  │
│ · repo-a     │  #142  CI ✗  review…    │ Tool: ci_get_…    │
│ · repo-b     │  #139  CI ✓  mergeable  │ LLM: flaky, …     │
│              │  #137  draft            │ [Approve] [Deny]  │
├──────────────┴─────────────────────────┴───────────────────┤
│ status: idle │ next cron 06:00 │ ollama ok │ mcp ok        │
└────────────────────────────────────────────────────────────┘
```

#### 视图与职责

| 视图 | 快捷键 | 内容 | 用户操作 |
|-----|--------|------|---------|
| **Dashboard** | `1` | 当日 Daily Digest 摘要；需关注 / 可忽略 / flaky 计数 | `r` 手动触发 digest；`Enter` 跳转 PR |
| **PR 列表** | `2` | 各 repo open PR（Store 中 `pr_snapshots` 缓存）；CI/review 图标 | 选中 PR → 详情；`t` 单 PR triage |
| **审批队列** | `3` | 待批准的 mutating 动作（rerun / backport） | `y` 批准、`n` 拒绝；批量可选 |
| **Logs** | `4` | Store `audit_log` + 当前会话流式事件 | 过滤级别；`/sqlite` 下支持关键词检索 |
| **Config** | `5` | 只读展示 `coworker.yaml`；连接状态（LLM / MCP / gh） | `e` 用 `$EDITOR` 打开配置文件 |
| **Flaky** | `6` | Store `flaky_tests` rollup：Top-N、按 repo 筛选、rerun 成功率 | Phase 2 完整报表；Phase 1 可先 CLI/`list` |
| **Chat** | `0` / `?` | 与秘书对话；展示 tool 摘要 | 输入自然语言；mutating 转 Approvals |

#### 交互原则

- **Mutating 默认进审批队列**，TUI 弹出确认；仅 `policy.auto_*` 为 true 且用户在 Config 显式开启时才跳过  
- **长输出在 TUI 内分页**，不 dump 原始 MCP 返回值；详情 pane 显示已 cap 的摘要  
- **SSH 友好**：纯终端、无 Web、单 binary；适合 jump host 上查看 Daily Work  
- **键盘优先**；鼠标可选（crossterm mouse support）

#### TUI 相关 crate

| Crate | 用途 |
|-------|------|
| `ratatui` | 布局、Table / List / Paragraph / Scrollbar |
| `crossterm` | 键盘、resize、alternate screen |
| `tokio::sync::broadcast` | Engine → TUI 事件流 |
| `unicode-width` | 中日文与 emoji 对齐（PR title 常含 emoji） |

**64K Token 预算分配（建议硬编码）：**

| 区块 | Token | 说明 |
|-----|-------|------|
| System + agent/skills | ≤ 4K | agent + 技巧 skills + TOOLS.md + repo 列表 |
| Tool schemas（lazy 模式） | ≤ 2K | 仅 3 个 meta tool |
| 对话 + tool results | ≤ 48K | 每轮 tool result 已被 mcp cap；Agent 侧再设 **单 PR 最多 3 次 tool 链** |
| 输出 reserved | ≥ 10K | digest + 推理余量 |

**多 PR 策略：一个 PR 一轮会话，禁止把 20 个 PR 的 logs 堆进同一 context。**  
Map-Reduce 用在 **跨 PR 汇总**：每 PR 先产出 ≤500 token 摘要，最后一轮 reduce 成 Daily Digest。

### 3. unistar-mcp 扩展（能力缺口，在 Go 里加 tool）

Harness 不应在 Rust/TS 里再包一层 gh。新能力 **进 unistar-mcp registry**，自动被 lazy 模式收录：

| 新 tool（按 [Tier B 工作流](#tier-b--需扩展-unistar-mcp-toolharness-模式不变) 优先级） | 作用 |
|---------------------|------|
| `pr_list_changed_files` | path + additions/deletions；Scheduler / pr-hygiene |
| `pr_list_stale` | N 天无更新 PR；pr-hygiene |
| `pr_get_diff` | cap patch；light-review、breaking-sniff |
| `pr_list_merged` | 按时间 merged PR；release-notes |
| `pr_post_comment` | mutating comment；comment-assist |
| `issue_list_open` / `issue_get` / `issue_add_label` | issue-triage |
| `alert_list_open` | Dependabot / code scanning；security-digest |
| `ci_list_runs` | duration + conclusion；ci-efficiency、main-guard |

遵循现有约定：`exec.go` 无 shell、`wrap()` 可行动错误、输出 hard cap、`registry.go` 注册。

---

## 与原方案的对照调整

### 保留

- **分而治之**：Per-PR agent session + 最终 reduce digest  
- **Token Budgeting**：硬预算 + 工具输出 cap（已在 mcp 实现，Harness 管 turn 级）  
- **输出清洗**：Digest 模板 + 去掉模型幻觉 HTML 标签  

### 降级 / 后移

- **本地 RAG + ChromaDB** → **不做**（与运维秘书定位无关；`light-review` diff Map-Reduce 已够用）  
- **AST / Tree-sitter 骨架** → Phase 3；Daily Work 的 CI triage 不需要读源码  
- **Webhook 驱动** → Phase 2  
- **Harness 内嵌 git diff** → 删除；统一走 `pr_get_diff` tool  

### 新增（原方案缺失）

- **MCP lazy mode 作为默认**  
- **Agent + Skill 分层**（任务在 `agents/`，技巧在 `skills/`；见 skill-agent-harness.md）  
- **Mutating action 审批 gate**（TUI 审批队列 + config 白名单 + 审计 log）  
- **本地 Store**（JSON / SQLite 可配置，无远程数据库）  
- **Flaky Test 账本**（incident 事件 + fingerprint 聚合，Phase 1 起攒数、Phase 2 报表）  
- **Workflow 插件模型**（`agents/` + `workflows` 配置 + `workflow_runs`）  
- **Chat 模式**（自然语言 + 自主 MCP 工具链；mutating 仍走审批）  
- **External CI 兜底**（`pr_get_status` 失败但 analyze 为空 → digest 提示看 PR 页面）

---

## 技术选型

| 组件 | 语言 | 理由 |
|-----|------|------|
| **unistar-mcp** | Go（已有） | gh/git 封装、cap、lazy registry 已成熟 |
| **unistar-coworker** | **Rust** | 单 binary：TUI + Engine + MCP client 一体；长驻 daemon；类型安全状态机 |

GitHub API 只在 Go mcp 里；Rust 负责 **TUI + 编排**，不重复封装 gh。

### Rust 依赖（建议）

| 职责 | Crate | 说明 |
|-----|-------|------|
| **TUI** | [`ratatui`](https://crates.io/crates/ratatui) + `crossterm` | 统一用户界面；Table/List/Scrollbar 做 PR 列表与日志 |
| MCP Client | [`rmcp`](https://crates.io/crates/rmcp) | stdio 子进程连 `unistar-mcp --lazy`；HTTP 连 `unistar-mcp http` |
| Async runtime | `tokio` | TUI 主循环 + Engine + MCP + LLM 同 runtime |
| 本地 LLM | `reqwest` | Ollama / vLLM OpenAI-compatible API |
| 配置 | `serde` + `serde_yaml` | `coworker.yaml` |
| **存储** | `serde_json` + **`rusqlite`**（feature `sqlite`） | `Store` trait；默认 JSON 目录，可选 SQLite 单文件 |
| 文件锁 | `fs4` 或 `fd-lock` | JSON 后端 + daemon/TUI 并发 |
| Cron | `tokio-cron-scheduler` | Engine 内调度；TUI 显示下次运行时间 |
| CLI | `clap` | 子命令见下 |
| 日志 / 审计 | `tracing` + `tracing-subscriber` | 同时写入文件；TUI Logs 视图订阅 |

**CLI 入口：**

| 命令 | 行为 |
|------|------|
| `unistar-coworker` | 默认：启动 **TUI + Engine** |
| `unistar-coworker run-once` | 无 TUI，跑默认工作流 `daily-work`（适合 systemd timer） |
| `unistar-coworker run-once --workflow release-duty` | 跑指定工作流 |
| `unistar-coworker daemon` | 无 TUI 长驻（Phase 2；TUI attach 共享 Store） |
| `unistar-coworker chat` | 交互式 REPL；LLM 自主调 MCP 工具（Phase 2+） |
| `unistar-coworker chat --once "…"` | 单轮自然语言命令后退出 |
| `unistar-coworker triage-pr --repo … --pr …` | 无 TUI 调试单 PR |
| `unistar-coworker report flaky --since 30d` | Flaky 报表 CLI（Phase 2） |
| `unistar-coworker report ci --since 7d` | CI 效率报表（Phase 2，需 ci-efficiency） |

### Crate 布局（ sketch ）

```
unistar-coworker/
├── Cargo.toml
├── coworker.yaml
├── agents/                  # 任务 SSOT（每工作流 / chat 一个 AGENT.md）
│   ├── daily-work/AGENT.md
│   ├── release-duty/AGENT.md
│   ├── chat/AGENT.md
│   └── …
├── skills/                  # 技巧 SSOT + _base/TOOLS.md
│   ├── ci-triage/SKILL.md
│   ├── digest-style/SKILL.md
│   ├── github-ops-tone/SKILL.md
│   ├── pr-merge/SKILL.md
│   └── _base/TOOLS.md
└── src/
    ├── main.rs
    ├── app/
    ├── tui/
    │   ├── …
    │   ├── chat.rs            # Chat 全屏 / 输入
    │   ├── release.rs         # [7] Phase 2
    │   └── issues.rs          # [8] Phase 2
    ├── engine/
    │   ├── scheduler.rs
    │   ├── workflows.rs       # workflow_id → agent / skills / schedule
    │   ├── prompt.rs          # compose_system_prompt
    │   ├── skill.rs           # MarkdownSpec, load_agent / load_skills / load_skill_with_base
    │   ├── chat.rs            # ChatSession + run_chat_turn
    │   └── mod.rs
    ├── agent/
    │   ├── loop.rs
    │   ├── budget.rs
    │   ├── chat_loop.rs       # MCP tool-use 循环（Chat / 可复用于 debug）
    │   └── reduce.rs
    ├── config.rs
    ├── store/
    │   ├── mod.rs           # Store trait
    │   ├── json.rs          # JSON 目录实现
    │   ├── sqlite.rs        # SQLite 实现（feature gate）
    │   └── model.rs         # Digest, PrSnapshot, Approval, FlakyIncident, …
    ├── llm/ollama.rs
    ├── mcp/client.rs
    └── output/
        └── export.rs        # Store → Markdown 文件（可选导出）
```

**任务规格**在 `agents/<workflow-id>/AGENT.md`；可复用 **技巧** `skills/ci-triage` 等。不在 Rust 里硬编码工作流步骤，保持迭代速度。

---

## 配置 sketch

```yaml
# coworker.yaml
llm:
  base_url: http://localhost:11434/v1
  model: gemma4:26b
  context_limit: 64000

mcp:
  command: unistar-mcp
  args: ["--lazy"]
  env:
    GH_TOKEN: ${GH_TOKEN}

# 本地存储：json（默认）或 sqlite，二选一
storage:
  backend: json                          # json | sqlite
  path: ~/.config/unistar-coworker/data  # json: 目录；sqlite: .db 文件路径
  # sqlite 可选：
  # wal: true                            # WAL 模式，daemon 并发更友好

schedule:
  daily_digest: "0 6 * * *"      # daily-work 默认；可被 workflows.*.schedule 覆盖
  ci_rescan: "0 */4 * * *"
  main_guard: "*/30 * * * *"       # main-guard Tier A

workflows:
  daily-work:
    enabled: true
    agent: agents/daily-work/AGENT.md
    skills:
      - skills/ci-triage/SKILL.md
      - skills/digest-style/SKILL.md
  release-duty:
    enabled: true
    agent: agents/release-duty/AGENT.md
    schedule: "0 9 * * 1-5"
  main-guard:
    enabled: true
    agent: agents/main-guard/AGENT.md
  review-radar:
    enabled: true
    agent: agents/review-radar/AGENT.md
  # issue-triage、security-digest 等 Tier B：enabled: false 直到 MCP tool 就绪

repos:
  - owner/repo-a
  - owner/repo-b

policy:
  auto_rerun_flaky: false      # mutating 默认关
  auto_backport: false
  max_prs_per_repo: 20
  max_agent_turns: 12
  max_tool_calls_per_pr: 5

flaky:
  record_real_bugs: true       # 是否也记录 llm_real（用于误报率统计）
  fingerprint_fallback: error  # test_name 缺失时用 error_signature 参与 fingerprint

output:
  export_digest_md: true       # 从 Store 额外导出 Markdown（TUI 主数据源仍是 Store）
  digest_export_path: ./digests/{date}.md
  # slack_webhook: ...         # 可选

chat:
  enabled: true
  agent: agents/chat/AGENT.md
  skills:
    - skills/github-ops-tone/SKILL.md
    - skills/ci-triage/SKILL.md
  max_turns: 8
  max_tool_calls: 6
  history_messages: 12
```

---

## 实施路线

路线图与 [产品功能目录](#产品功能目录) 的 Tier / Phase 对齐。

### Phase 1 — Daily Work MVP（2–3 周）

**工作流：** `daily-work`（Tier A）

- [x] **`Store` trait**：JSON（默认）+ SQLite（feature `sqlite`）  
- [x] 持久化：digests、pr_snapshots、approvals、audit_log、flaky_*、**workflow_runs**  
- [x] **`engine/workflows.rs`**：workflow_id → agent + skills 加载  
- [x] triage 管线：`record_flaky_incident` + rerun rollup  
- [x] Rust：`engine` + MCP stdio + Ollama + cron（设计中的 `rmcp`；当前为 subprocess MCP client）  
- [x] TUI 骨架：Dashboard + PR 列表 + Logs + 审批队列  
- [x] Agent：`agents/daily-work/` + `skills/ci-triage`  
- [x] `unistar-coworker` 默认 TUI；`run-once [--workflow daily-work]`

### Phase 2 — Tier A 补全 + Tier B 起步（4–6 周）

**工作流：** `release-duty`、`main-guard`、`review-radar`、`flaky-govern`、`oncall-handoff`；`issue-triage`（MCP 就绪后）

- [x] Tier A agents + cron：`release-duty`、`main-guard`、`review-radar`、`flaky-govern`、`oncall-handoff`、`my-pr-brief`、`ci-efficiency`  
- [x] Store：`backport_queue`、`main_alerts`、`issue_snapshots`  
- [x] TUI：`[7] Release` backport 队列；`[8] Issues`（issue-triage 启用时）  
- [x] TUI：`[6] Flaky` 完整报表（repo 筛选、7/30d、quarantine 建议、用户改判）  
- [x] CLI：`report flaky`；`report oncall`；`report ci`；`triage-pr`  
- [x] mcp（Tier B 首批）：`pr_list_changed_files`、`issue_*`、`ci_list_runs`、`alert_list_open`  
- [x] `daemon`；`store migrate`（json↔sqlite）
- [x] TUI attach：`--attach` 共享 Store（JSON 文件锁 `.coworker.lock`）  
- [x] 可选 Slack 通知（TUI 仍为主界面）  
- [x] **Chat MVP（CLI）**：`chat` / `chat --once` + `agents/chat` + `chat_sessions` Store  
- [x] **Chat TUI 视图**：`[0]` / `?` 全屏对话 + tool 轨迹摘要

### Phase 3 — Tier B 深化 + Tier C（按需）

**工作流：** `security-digest`、`release-notes`、`comment-assist`、`pr-hygiene`、`light-review` 等

- [x] 剩余 Tier B MCP tools（`pr_list_stale`、`pr_get_diff`、`pr_post_comment`、`pr_list_merged`）+ workflow agents  
- [x] TUI `[8] Issues`（issue-triage）  
- [x] Dashboard 安全告警区块（security-digest）  
- [x] Tier C：`light-review` Map-Reduce、`regression-link`、`breaking-sniff`  
- [x] 平台：规则引擎、Playbook few-shot、多 MCP（可选 stub）  
- [x] Chat：Store 只读摘要、`--list-sessions`；**禁止** Chat fork 其他 agent（仅 tools + skills）
- [x] ~~可选轻量 RAG~~ **明确不做**（`light-review` 不引入 symbol 检索 / 向量索引）  

---

## 一句话总结

**unistar-coworker 是带 TUI 的本地 GitHub 运维秘书 + 轻量 coding Chat：** Workflow/cron（Daily Work、Release 值班…）共用 Engine + Store + MCP；**Chat 默认**在 `chat.workspace` 读/搜/改代码；GitHub MCP 按需 lazy；mutating 走审批；不做 Web UI、不做无约束自主程序员。
