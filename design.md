> 本地部署的 Gemma4:26B（64K context），基于 **unistar-mcp** 的 **unistar-coworker** —— 带 TUI 的 **GitHub 运维秘书** Harness。  
> v1 旗舰工作流是 **Daily Work**；同一套 Engine + Store + Skill 骨架可扩展更多工作流（见 [产品功能目录](#产品功能目录)）。

## 产品定位

| 是 | 不是 |
|----|------|
| 读 MCP cap 过的 GitHub/CI 信号，本地 LLM 辅助判断 | 替代 GitHub Actions / 自建 CI runner |
| 记账、digest、报表、代写草稿 | 无审批全自动 merge / 大范围改代码 |
| TUI 内审批 mutating 动作 | 云端 SaaS、Web Dashboard |
| 多工作流共用 Scheduler + Agent + Store | 一次性脚本合集 |

**定位一句话：GitHub 运维秘书** —— 在 64K 本地模型约束下，做「该看啥、该干啥、帮你想好草稿、等你拍板」。

## 设计前提：不要重复造轮子

原方案把 Harness 设计成「Webhook → 上下文压缩 → RAG → Map-Reduce → gh 回写」的 monolith。  
**unistar-mcp 已经解决了其中最难、最 GitHub 耦合的部分**——在 64K 模型上可用的、上下文节俭的工具层：

| 原方案模块 | unistar-mcp 现状 | 结论 |
|-----------|-----------------|------|
| Context Squeezer（diff / 日志脱水） | `gh`/`git` 只取必要 JSON 字段；CI 日志 error-extract + ~6KB cap；`pr_list_open` 默认 limit=20 | **复用，Harness 不再自己做 gh 封装** |
| 工具 schema 占满 context | `--lazy` 模式：仅暴露 `tool_list` / `tool_describe` / `tool_call` 三个 meta tool | **64K 模型默认开 lazy** |
| Action & Feedback | `ci_rerun_workflow`、`pr_create_backport` 等 mutating tools；无 shell、防 double-execute | **复用；Harness 负责审批 gate** |
| 工作流编排 | `pr-ci-triage` skill 已定义工具链顺序 | **Harness 加载 skill 作为 system prompt** |
| AST 骨架 / 代码 RAG | **未覆盖** | 留给 Phase 3（PR 代码审查），Daily Work 不优先 |
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

4. **`ci_rerun_workflow`** — 仅对 skill 判定为 flaky 且用户在 config 中开启 `auto_rerun_flaky: true` 的 run 执行  
5. **`pr_create_backport`** — 对「已 merge 且 label 含 `needs-backport`」的 PR 按 release 分支规则创建 backport（或仅生成建议列表）

### v1 明确不做（其他工作流后续迭代）

- 行级 Code Review / 大 diff 摘要 → 工作流 `light-review`（Phase 3）  
- Issue 自动回复 / 打标 → 工作流 `issue-triage`（Phase 2）  
- 全库 RAG → 仅 `light-review` 可选（Phase 3）

---

## 产品功能目录

所有功能共享同一 **Workflow 执行模型**，差异只在 Skill、调度、cron 与 Store 写入项：

```
cron / 手动 / Webhook(Phase 2)
    → Scheduler 规则过滤（零 LLM）
    → 任务队列 (workflow_id, repo, subject, …)
    → Agent Loop（加载对应 skill + token 预算）
    → MCP 按需拉取 cap 信号
    → 本地 LLM 判断 / 摘要
    → Store 持久化
    → TUI 展示 + 可选 mutating 审批
```

### Workflow 配置约定

```yaml
# coworker.yaml 片段
workflows:
  daily-work:
    enabled: true
    skill: skills/daily-work/SKILL.md
    schedule: "0 6 * * *"        # 可覆盖全局 schedule
    mutating: [rerun_flaky, backport]  # 需 TUI 审批
  release-duty:
    enabled: true
    skill: skills/release-duty/SKILL.md
    schedule: "0 9 * * 1-5"      # 工作日 9:00
    mutating: [backport]
```

每个工作流对应 **一个 Skill 目录**（与 Claude Code 共用 Markdown SSOT），Engine 按 `workflow_id` 加载 prompt 与工具链说明。

### Tier A — 现有 MCP 工具即可（优先落地）

仅 Harness + Skill + Store；**不强制**扩展 unistar-mcp。

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

### Tier B — 需扩展 unistar-mcp tool（Harness 模式不变）

新 tool 进 Go registry（cap + lazy）；Coworker 加 Skill + Store 表。

| ID | 名称 | 依赖新 tool | 做什么 |
|----|------|-------------|--------|
| **`issue-triage`** | Issue 分诊 | `issue_list_open`、`issue_get`；可选 `issue_add_label` | 未处理 issue 分类、重复聚类（标题相似度 Harness 侧）、优先级 digest |
| **`pr-hygiene`** | PR 卫生 | `pr_list_stale`、`pr_list_changed_files` | N 天无更新提醒；docs-only 过滤；超大 PR 警告 |
| **`security-digest`** | 安全告警简报 | `alert_list_open`（Dependabot/code scanning） | Critical/High 摘要 + LLM 一句影响面 |
| **`release-notes`** | Release notes 草稿 | `pr_list_merged --since` | Map-Reduce 生成 changelog 草案，人改后贴 Release |
| **`ci-efficiency`** | CI 效率报表 | `ci_list_runs`（duration、conclusion 一行） | 最慢 workflow、失败率趋势（Store 聚合，类 flaky） |
| **`comment-assist`** | Comment 助手 | `pr_post_comment` | CI 失败时生成 comment 草稿 → 审批后发送 |
| **`merge-health`** | Merge 队列健康 | `pr_get_status` + branch protection 字段 | 「理论上可 merge 但被 blocked」及原因清单 |

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
| **Skill 目录** | `skills/<workflow-id>/SKILL.md` 即 SSOT；与 Claude Code 项目 skill 可软链共用 |
| **Playbook 回放** | Store 存成功 triage 的 transcript → 作 few-shot 提升弱模型 flaky 判准率 |
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

Phase 1 保持 6 个 tab；Phase 2 起 `[7][8]` 随工作流启用出现。

---

## 架构：TUI + 引擎 + MCP

**TUI 是唯一用户界面**——配置、调度、digest 浏览、PR triage、mutating 审批、Agent 实时日志，全部在终端内完成；不提供 Web Dashboard。

```
┌──────────────────────────────────────────────────────────────────┐
│  unistar-coworker TUI (ratatui)          ← 统一用户界面           │
│  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────────────┐     │
│  │ Dashboard│ │ PR 列表  │ │ 审批队列 │ │ Agent 日志/Transcript│  │
│  └────┬─────┘ └────┬─────┘ └────┬─────┘ └────────┬─────────┘     │
│       └────────────┴────────────┴────────────────┘               │
│                         │ mpsc / broadcast (AppEvent)            │
├─────────────────────────┼────────────────────────────────────────┤
│  Core Engine (tokio)    ▼                                        │
│  ┌─────────────┐  ┌──────────────┐  ┌────────────────┐           │
│  │ Scheduler   │  │ Agent Loop   │  │ Token Budget   │           │
│  │ (cron/      │→ │ (local LLM   │←→│ (64K 硬预算)    │           │
│  │  manual)    │  │  + skills)   │  │                │           │
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
- 标记：`action_required` 不算 failure（skill 已说明）  
- 输出：`(workflow_id, repo, pr_number, task_type)` 任务队列

### 2. Agent Loop（大脑，薄编排）

Harness 本身 **不解析 diff、不调用 gh**，只：

1. 连接本地推理端点（Ollama / vLLM / llama.cpp），Gemma4:26B @ 64K  
2. 连接 `unistar-mcp`（推荐 `unistar-mcp --lazy` stdio 子进程，或 `http :8080` 多 worker 共享）  
3. 按任务 **`workflow_id`** 加载对应 skill（如 `skills/daily-work/SKILL.md`）+ 任务 prompt  
4. 运行受限 MCP Agent 循环（max turns、max tool calls、timeout）  
5. 汇总 transcript → **写入 Store** → 推送到 TUI → 可选导出 Markdown / Slack

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
| System + skill | ≤ 4K | skill 全文 + repo 列表 config |
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

- **本地 RAG + ChromaDB** → Phase 3，仅当要做「跨 repo 代码语义搜索」或「大 PR review」  
- **AST / Tree-sitter 骨架** → Phase 3；Daily Work 的 CI triage 不需要读源码  
- **Webhook 驱动** → Phase 2  
- **Harness 内嵌 git diff** → 删除；统一走 `pr_get_diff` tool  

### 新增（原方案缺失）

- **MCP lazy mode 作为默认**  
- **Skill 作为工作流 SSOT**（与 Claude Code 共用同一份 `pr-ci-triage`）  
- **Mutating action 审批 gate**（TUI 审批队列 + config 白名单 + 审计 log）  
- **本地 Store**（JSON / SQLite 可配置，无远程数据库）  
- **Flaky Test 账本**（incident 事件 + fingerprint 聚合，Phase 1 起攒数、Phase 2 报表）  
- **Workflow 插件模型**（Skill 目录 + `workflows` 配置 + `workflow_runs`）  
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
| `unistar-coworker triage-pr --repo … --pr …` | 无 TUI 调试单 PR |
| `unistar-coworker report flaky --since 30d` | Flaky 报表 CLI（Phase 2） |
| `unistar-coworker report ci --since 7d` | CI 效率报表（Phase 2，需 ci-efficiency） |

### Crate 布局（ sketch ）

```
unistar-coworker/
├── Cargo.toml
├── coworker.yaml
├── skills/
│   ├── daily-work/SKILL.md
│   ├── release-duty/SKILL.md
│   ├── main-guard/SKILL.md
│   └── …                    # 每工作流一个目录
└── src/
    ├── main.rs
    ├── app/
    ├── tui/
    │   ├── …
    │   ├── release.rs         # [7] Phase 2
    │   └── issues.rs          # [8] Phase 2
    ├── engine/
    │   ├── scheduler.rs
    │   ├── workflows.rs       # workflow_id → skill / schedule / mutating
    │   └── mod.rs
    ├── agent/
    │   ├── loop.rs
    │   ├── budget.rs
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

Skill 按 **工作流目录** 组织（`skills/<workflow-id>/SKILL.md`）；`daily-work` 可软链 unistar-mcp 的 `pr-ci-triage`。不在 Rust 里硬编码工作流，保持迭代速度。

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
    skill: skills/daily-work/SKILL.md
  release-duty:
    enabled: true
    skill: skills/release-duty/SKILL.md
    schedule: "0 9 * * 1-5"
  main-guard:
    enabled: true
    skill: skills/main-guard/SKILL.md
  review-radar:
    enabled: true
    skill: skills/review-radar/SKILL.md
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
```

---

## 实施路线

路线图与 [产品功能目录](#产品功能目录) 的 Tier / Phase 对齐。

### Phase 1 — Daily Work MVP（2–3 周）

**工作流：** `daily-work`（Tier A）

- [ ] **`Store` trait**：JSON（默认）+ SQLite（feature `sqlite`）  
- [ ] 持久化：digests、pr_snapshots、approvals、audit_log、flaky_*、**workflow_runs**  
- [ ] **`engine/workflows.rs`**：workflow_id → skill 加载  
- [ ] triage 管线：`record_flaky_incident` + rerun rollup  
- [ ] Rust：`engine` + `rmcp` + Ollama + cron  
- [ ] TUI 骨架：Dashboard + PR 列表 + Logs + 审批队列  
- [ ] Skill：`skills/daily-work/`（可软链 `pr-ci-triage`）  
- [ ] `unistar-coworker` 默认 TUI；`run-once [--workflow daily-work]`

### Phase 2 — Tier A 补全 + Tier B 起步（4–6 周）

**工作流：** `release-duty`、`main-guard`、`review-radar`、`flaky-govern`、`oncall-handoff`；`issue-triage`（MCP 就绪后）

- [ ] Tier A Skills + cron：`release-duty`、`main-guard`、`review-radar`  
- [ ] Store：`backport_queue`、`main_alerts`  
- [ ] TUI：`[6] Flaky` 完整报表；`[7] Release` backport 队列  
- [ ] CLI：`report flaky`；`oncall-handoff` 导出  
- [ ] mcp（Tier B 首批）：`pr_list_changed_files`、`issue_*`、`ci_list_runs`  
- [ ] `daemon` + TUI attach；`store migrate`  
- [ ] 可选 Slack 通知（TUI 仍为主界面）

### Phase 3 — Tier B 深化 + Tier C（按需）

**工作流：** `issue-triage`、`security-digest`、`release-notes`、`ci-efficiency`、`comment-assist`、`light-review` 等

- [ ] 剩余 Tier B MCP tools + Skills  
- [ ] TUI `[8] Issues`；Dashboard 安全告警区块  
- [ ] Tier C：`light-review` Map-Reduce、`regression-link`  
- [ ] 平台：规则引擎、Playbook few-shot、多 MCP（可选）  
- [ ] 可选轻量 RAG（仅 light-review 变更 symbol）

---

## 一句话总结

**unistar-coworker 是带 TUI 的本地 GitHub 运维秘书：** 多工作流（Daily Work、Release 值班、Main 红线、Flaky 治理…）共用 Engine + Store + Skill；unistar-mcp 脱水取数、Gemma 64K 内判断、TUI 审批后动手；JSON/SQLite 本地记账；不做 Web UI、不做自主程序员。
