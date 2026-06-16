# Skill → Agent → Harness

> 现状诊断 + 目标分层 + 渐进迁移计划  
> 与 `base-tool-plan.md`（工具 SSOT）互补：工具在 harness/MCP，**技巧**在 skill，**任务编排**在 agent。

## 现状：名实不符

| 路径 | 文件名 | 实际内容 | 真正角色 |
|------|--------|----------|----------|
| `skills/chat/` | SKILL.md | JSON schema、tool 白名单、approval 规则、when-to-use | **Agent** |
| `skills/daily-work/` | SKILL.md | 逐步流程 + tool 表 + 分类规则 | **Agent** + 内嵌 skill 片段 |
| `skills/merge-health/` | SKILL.md | 4 步扫描说明 | **Agent 文档**（逻辑已在 Rust） |
| `skills/light-review/` | SKILL.md | 一句话 procedure | **Agent stub** |
| `src/agent/*.rs` | — | MCP 调用、循环、budget、Store | **Harness** |
| `src/llm/client.rs` | — | classify 的 verdict 定义、字段长度 | **Skill**（硬编码在 Rust 里） |

### 三类内容混在一个文件里

以 `skills/daily-work/SKILL.md` 为例：

```
┌─────────────────────────────────────────┐
│  Per-repo workflow (步骤 1–4)           │  → Agent（做什么、顺序）
│  Tools 表                               │  → Agent / base tools 引用
│  Rules (flaky vs policy, action_req…)   │  → Skill（怎么判断）
└─────────────────────────────────────────┘
```

### 代码里实际怎么用

| 消费者 | 读 SKILL.md 的方式 |
|--------|-------------------|
| **Chat** | 整篇 `skill.body` 进 system prompt |
| **Triage / classify** | `skill.body` + `client.rs` 里又写一遍 verdict 规则 |
| **merge-health 等** |  mostly `IncrementalDigest::begin(skill)` 标题；**编排全在 Rust** |
| **Scheduler** | 只记 path，不解析语义 |

所以：**Skill 目录是 SSOT 的名字，Agent 规格是 SSOT 的实态；真正可复用的「技巧」反而散落在 SKILL 的 Rules 段和 `llm/client.rs` 里。**

---

## 目标三层

```
┌──────────────────────────────────────────────────────────────┐
│  Skill — 教模型「怎么做对」                                     │
│  · 领域技巧、判断标准、反模式、few-shot 风格                    │
│  · 可跨 agent 复用（ci-triage、pr-merge、digest-style）        │
│  · 不含 cron、不含 JSON action schema、不含具体 tool 调用顺序   │
└────────────────────────────┬─────────────────────────────────┘
                             │ compose at load time
┌────────────────────────────▼─────────────────────────────────┐
│  Agent — 这次任务「要达成什么」                                 │
│  · 角色、目标、输出格式、tool/workflow 策略、scope/budget       │
│  · Chat secretary / daily-work runner / merge-health scanner   │
│  · 可引用 skills[] + tools.base                                  │
└────────────────────────────┬─────────────────────────────────┘
                             │ invoked by
┌────────────────────────────▼─────────────────────────────────┐
│  Harness — 确定性执行（Rust）                                   │
│  · MCP、Store、审批、并发、budget 硬限制                        │
│  · 不依赖模型记住步骤；agent 文档给人 + 给 ad-hoc LLM  loop     │
└──────────────────────────────────────────────────────────────┘
```

### 判断标准（写文件时自问）

| 问题 | Skill | Agent | Harness |
|------|-------|-------|---------|
| 换 workflow 还想复用？ | ✓ | ✗ | — |
| 描述「第 1 步 list PR，第 2 步 triage」？ | ✗ | ✓ | Rust 若已写死则可省略 |
| 必须 100% 执行、不能靠模型？ | ✗ | ✗ | ✓ |
| JSON `action: tool` schema？ | ✗ | ✓ (chat) | ✓ (parser) |
| 「flaky vs real vs policy 怎么分」？ | ✓ | 可引用 skill | heuristic fallback |

---

## 目录布局（目标）

```
skills/                          # 技巧 SSOT
  ci-triage/
    SKILL.md                     # 分类标准、日志阅读、policy 反例
  pr-merge/
    SKILL.md                     # merge blockers / review 状态解读
  digest-style/
    SKILL.md                     # ops digest 写法、简洁度
  github-ops-tone/
    SKILL.md                     # 不臆造、中英文、秘书语气

agents/                          # 任务 SSOT（现 skills/ 工作流迁移来）
  chat/
    AGENT.md
  daily-work/
    AGENT.md
  merge-health/
    AGENT.md

skills/_base/
  TOOLS.md                       # 见 base-tool-plan.md

src/agent/                       # Harness（保持 Rust 模块名，不必改名）
  triage.rs
  chat_loop.rs
  loop.rs
  ...
```

**Chat 特例：** `agents/chat/AGENT.md` 管 loop 合约；`skills/github-ops-tone` + `skills/ci-triage` 等按需注入。

---

## Prompt 组装

```rust
// 目标 API（engine/prompt.rs）
pub struct PromptBundle {
    pub agent: AgentSpec,
    pub skills: Vec<SkillSpec>,
    pub tools_doc: String,   // from _base/TOOLS.md + config
    pub runtime_context: String, // repos, store snapshot, …
}

pub fn compose_system_prompt(bundle: &PromptBundle) -> String {
    format!(
        "{}\n\n## Techniques\n{}\n\n## Tools\n{}\n\n## Context\n{}",
        bundle.agent.body,
        join_skills(&bundle.skills),
        bundle.tools_doc,
        bundle.runtime_context,
    )
}
```

**Triage classify**  today:

```
system = skill_body + hardcoded verdict block in client.rs
```

**Target:**

```
system = compose(ci-triage skill) + task-specific agent snippet (optional)
// 删除 client.rs 里重复的 verdict 段落，单一 SSOT 在 skills/ci-triage
```

---

## 配置 sketch

```yaml
workflows:
  daily-work:
    enabled: true
    agent: agents/daily-work/AGENT.md
    skills:
      - skills/ci-triage/SKILL.md
      - skills/digest-style/SKILL.md
    schedule: "0 6 * * *"

  merge-health:
    enabled: true
    agent: agents/merge-health/AGENT.md   # 薄：意图 + 输出格式
    skills:
      - skills/pr-merge/SKILL.md
    # 编排主要在 harness/merge_health.rs

chat:
  agent: agents/chat/AGENT.md
  skills:
    - skills/github-ops-tone/SKILL.md
    - skills/ci-triage/SKILL.md
  preferred_tools: []   # 仍见 base-tool-plan

# 兼容期（已完成）
# 旧 config `skill:` 已移除；任务 SSOT 为 agents/<id>/AGENT.md
```

---

## 内容迁移示例

### 从 `skills/daily-work/SKILL.md` 拆出

**`skills/ci-triage/SKILL.md`**（技巧）

```markdown
---
name: ci-triage
description: Classify CI failures — flaky, real bug, or policy gate.
---

## Verdicts
- flaky: transient infra/timeouts; rerun may pass
- real: test/build failure in the PR
- policy: approvals, changelog, labels — not engineering work
- unknown: insufficient logs on this page

## Rules
- `action_required` is approval-waiting, not a code failure
- External CI: status red but no Actions runs → say so explicitly
- Read summary before full logs; page through logs before concluding unknown
```

**`agents/daily-work/AGENT.md`**（行动）

```markdown
---
name: daily-work
description: Morning triage digest across configured repos.
skills: [ci-triage, digest-style]
---

## Goal
Produce daily digest: open PRs, classify failing CI, split attention/flaky/policy.

## Procedure
1. `pr_list_open` per repo
2. Failing CI → triage harness (`triage_pr`) per PR
3. Publish digest to Store

## Scope
- One PR per triage session for log context
- Mutating tools → approval only
```

**Harness** — `loop.rs` / `triage.rs` 不变，继续 enforce budget。

### `skills/chat/SKILL.md` → `agents/chat/AGENT.md`

移走：

- Preferred tools 表 → `_base/TOOLS.md` + config
- JSON action schema → AGENT.md（这是 agent 合约）
- 「Single PR → pr_get_overview」→ 可留 agent 的 routing hints，或极薄

移入 **`skills/github-ops-tone`**：

- 不臆造 PR/CI
- 用户语言回复
- meta 问题 ≤8 bullets

移入 **`skills/ci-triage`**（chat 查 CI 时复用）：

- summary before logs
- flaky vs real 解释给用户

---

## 实现阶段

### Phase 0 — 命名与文档（无破坏）

- [x] 本文档
- [x] `design.md` 增加三层 glossary，标注当前 `skills/` = 过渡态
- [x] README + `skills/README.md`：workflow skill 已迁移为 agent

### Phase 1 — Loader 双轨

- [x] `engine/prompt.rs`: `PromptBundle`, `compose_system_prompt`, `load_workflow_spec`
- [x] `engine/skill.rs`: `load_agent`, `load_skills`, frontmatter `skills:[]`
- [x] `compose_system_prompt()` 用于 chat + classify
- [x] Config: `agent` + `skills[]`（已移除 `skill:` fallback）

### Phase 2 — 抽第一个共享 skill

- [x] 新建 `skills/ci-triage/SKILL.md`（从 daily-work Rules + client.rs verdict 段合并）
- [x] `classify_log_page` 改用 `compose_classify_prompt`；删 client.rs 重复段落
- [x] daily-work / chat config 引用 `ci-triage`

### Phase 3 — Chat 拆分

- [x] `agents/chat/AGENT.md`
- [x] `skills/github-ops-tone/SKILL.md`
- [x] `chat_loop.rs` 用 `PromptBundle`

### Phase 4 — Workflow 目录迁移

- [x] `agents/<id>/AGENT.md` 从原 workflow 规格拆/agent 化（含 Goal/Procedure/Scope）
- [x] 纯 harness workflow → 薄 AGENT.md + 引用技巧 skills
- [x] `coworker.example.yaml` 更新
- [x] 删除 `skills/<workflow-id>/` stub；`skills/` 仅保留技巧 + `_base`

### Phase 5 — 可选 discovery

- [x] `unistar-coworker agents list` / `skills list` CLI
- [x] Agent 可声明 `skills:` frontmatter，loader 自动解析

---

## 与 base-tool-plan 的关系

| 层 | 文档 | 内容 |
|----|------|------|
| Tools | `skills/_base/TOOLS.md` + harness MCP | 工具名与用途 |
| Skill | `skills/*/SKILL.md` | 使用工具的**技巧** |
| Agent | `agents/*/AGENT.md` | 用哪些 skill/tool、任务目标 |
| Harness | `src/agent/*.rs` | 实际调用 |

Chat preferred_tools → **Agent** 配置 + **Tools** 文档，不是 skill。

---

## 非目标

- 不把 harness 逻辑搬进 markdown（merge-health 仍 Rust 扫描）
- 不追求每个 workflow 一个 LLM agent loop（daily-work 仍是 harness 驱动 + 局部 classify）
- 不重命名 `src/agent/` 模块（Rust 侧继续叫 agent = harness 实现，文档区分即可）

---

## 相关

- `base-tool-plan.md` — MCP vs harness tools
- `design.md` — Chat 模式、Agent Loop
- `src/engine/skill.rs` — `MarkdownSpec`, `load_agent`, `load_skills`, `load_skill_with_base`, `read_base_tools`
