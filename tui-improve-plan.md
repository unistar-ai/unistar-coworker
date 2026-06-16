# TUI 优化计划

> 对标 Claude Code / OpenCode / Aider 等终端 Agent 产品，结合 **unistar-coworker** 定位（GitHub 运维秘书、非编程 Agent），给出可落地的 TUI 演进路线。  
> 与 `design.md`（架构 SSOT）、`skill-agent-harness.md`（Agent 分层）互补；本文只谈 **终端体验**。

---

## 1. 背景与目标

### 1.1 产品约束（不变）

| 约束 | 含义 |
|------|------|
| **TUI 是唯一 GUI** | 不做 Web Dashboard；SSH / jump host 友好 |
| **运维秘书，不是 Claude Code** | 主场景是 PR/CI/digest/审批，不是改 repo 源码 |
| **本地小模型** | Gemma @ 64K；UI 要能容忍慢推理、JSON action、偶发截断 |
| **Engine ↔ TUI 解耦** | `AppEvent` + `AppState`；Agent 不阻塞渲染 |
| **单 binary Rust** | 当前栈：`ratatui` + `crossterm`；不引入 Node/React 运行时 |

### 1.2 UX 目标（借鉴 Claude Code 的「Observable Autonomy」）

> **Trust but verify in real time** — Agent 可自主调工具，但用户应能在 3 秒内看出「走偏了」并 `Ctrl+C` / 取消，而不是等 6 次失败 tool 后才看到半截回复。

对本项目具体化为：

1. **实时可见**：LLM 思考、tool 参数、tool 进度、assistant 增量输出，尽量流式呈现  
2. **少打断**：mutating 才进审批；只读 tool 不打 modal  
3. **可扫读**：Chat 里 PR `#N`、CI 状态、列表结构一眼能读；Detail/Digest 同样受益  
4. **键盘优先**：笔记本无 PgUp/PgDn、Chat 与 Tab 快捷键不打架（已部分做）  
5. **长会话稳定**：虚拟滚动 / 行高缓存，避免 300 行 chat 历史拖慢 redraw  

---

## 2. 业界对标摘要

### 2.1 Claude Code（闭源，终端 REPL 标杆）

| 维度 | 做法 | 可借鉴点 |
|------|------|----------|
| **渲染** | Fork Ink + React 19 + Yoga Flexbox；双缓冲 + ANSI diff；~60fps | 组件化、按 cell diff 减闪烁；**不必 fork**，ratatui 已有 buffer |
| **流式** | `async function*` + `Stream<T>`；Token → React state → 增量 redraw | **assistant 逐 token / 逐段** 显示，而非 turn 结束才 push 一行 |
| **Markdown** | `StreamingMarkdown`：稳定前缀不 re-parse，只 parse 增长尾部 | Chat 长回复必备；我们当前是 **整段 render** |
| **Tool UI** | 每 tool 4 种 render；`ToolUseLoader` 状态点；并行 tool 同步 blink | 参数可见、进行中/成功/失败颜色一致；Bash 可 **progress stream** |
| **Spinner** | 多模式：`requesting` / `thinking` / `tool-use`；stall 变红 | 区分「等 API」vs「等 MCP」vs「模型在想」 |
| **权限** | Permission dialog + 200ms 防误触；classifier shimmer | 对应我们的 **Approvals tab**；可加确认延迟 |
| **滚动** | `VirtualMessageList` + 行高缓存；鼠标选中/复制 | Chat 历史 >100 条时必需 |
| **哲学** | 工具执行与模型输出 **可并行**（StreamingToolExecutor） | 我们只读 tool 可并行；mutating 仍串行 + 审批 |

参考：[how-claude-code-works — Ch.14 User Experience](https://github.com/Windy3f3f3f3f/how-claude-code-works/blob/main/en/docs/12-user-experience.md)

### 2.2 OpenCode（开源，全屏 TUI Agent）

| 维度 | 做法 | 可借鉴点 |
|------|------|----------|
| **架构** | Agent core + 多前端（TUI/CLI/Web/Desktop）；Bubble Tea / Solid | **client-server attach** 思路 → 我们已有 `daemon` + `--attach` |
| **布局** | 左对话 + 右侧栏（token、cost、LSP）；底栏 prompt + 进度条 | 右侧栏可映射为 **MCP/LLM/Store 健康 + token 预算** |
| **Tool 符号** | `→` Read / `←` Write；footer `■■■⬝⬝` 进度 | 我们可用 `→/✓/✗` + **footer 进度**（workflow / chat turn） |
| **模式** | Build / Plan 切换；`esc interrupt` 提示 | Chat busy 时显式 **Esc 取消 turn**（未做） |

### 2.3 Aider（极简 CLI）

| 维度 | 做法 | 可借鉴点 |
|------|------|----------|
| **形态** | REPL 式、非全屏；git auto-commit | **不适合**我们做主 UI；但 `/clear`、显式 context 控制值得 Chat 借鉴 |
| **优势** | 上下文边界清晰、成熟稳定 | Chat 可提供 **`/clear` `/sessions`** 的 TUI 等价物 |

### 2.4 Cursor / Codex CLI（IDE 系）

| 维度 | 做法 | 可借鉴点 |
|------|------|----------|
| **Tool calling** | API 级 `tool_use`，非 JSON 字符串 | 长期：Ollama native tools；短期：TUI 只关心 **展示** |
| **并行 tool** | 同轮多 tool | 与 Claude Code 类似；我们只读 MCP 可批跑 |
| **Diff 预览** | 改文件前 diff | **非目标**（我们不改代码）；PR diff 可在 Detail 只读预览 |

### 2.5 对标结论（我们选哪条路）

```
                    功能丰富
                       ▲
         OpenCode ●    │    ● Claude Code
                       │
    unistar-coworker   │         （目标区间：OpenCode 的信息密度
         （现在）●     │          + Claude Code 的 tool 透明度）
                       │          − 代码 diff / LSP
                       │
              Aider ●  │
                       └──────────────────────► 实现复杂度
```

**不 fork Ink、不重写 React TUI**；在 ratatui 上吸收 **流式、虚拟滚动、tool 透明、StreamingMarkdown** 四类能力即可覆盖 80% 体感差距。

---

## 3. 现状盘点（v0.3+ 本地）

### 3.1 已有

| 能力 | 实现 | 差距 |
|------|------|------|
| 多 Tab 壳 | `src/tui/mod.rs` — Dashboard/PRs/Approvals/…/Chat | Detail 仍纯文本，无 Markdown |
| Chat 全屏 | `src/tui/chat.rs` | 无 token 级流式；busy 仅 `◌ thinking` |
| Tool 进度 | `ChatProgress` → `→/✓` 行 | 无参数折叠/展开；无并行分组 |
| Markdown | `pulldown-cmark` → ratatui `Line` | 无 streaming；无表格；链接仍冗长 |
| 主题 | `theme.rs` Catppuccin 风 | 无 light theme；无用户配置 |
| 滚动 | `chat_scroll_from_bottom` + 多种快捷键 | 非虚拟列表；行数估算是 O(n) |
| 输入 | 终端光标 + Input 框 | 无 multiline；无 history ↑ |
| 事件 | `broadcast::AppEvent` | Chat 无 `ChatTokenDelta` 事件 |
| CLI  parity | `main.rs` chat `--once` 全量 tool 输出 | TUI 与 CLI 行为不一致处仍可能存在 |

### 3.2 主要痛点（用户反馈 + 代码审查）

1. **Chat 回复「一次性出现」** — 长 Markdown 列表仍像 dump，缺少打字机/流式感  
2. **Markdown 不够好看** — 表格、嵌套列表、长 URL、代码块换行仍粗糙  
3. **Tool 循环不可见** — 重复 `pr_get_overview` 直到 budget 耗尽（Harness 问题 + UI 未突出「相同失败」）  
4. **Detail / Digest 与 Chat 体验割裂** — 右侧大段 `body_md` 无渲染  
5. **非 Chat Tab 仍偏「调试界面」** — 列表信息密度低、缺图标/状态语义  
6. **无可取消的 long-running turn** — workflow/chat 只能等或 `Ctrl+C` 杀进程  

---

## 4. 设计原则（写入后续 PR 的验收标准）

1. **Glass box tool loop**：每次 tool 必须可见 `name + 关键 args + 耗时 + ok/err`；相同 args 连续失败 ≥2 次要在 UI **警告**（Harness 侧拦截，UI 侧高亮）。  
2. **Progressive disclosure**：默认折叠 tool 结果（一行摘要）；`Enter` / `o` 展开详情（cap 6000 字符分页）。  
3. **Streaming first for Chat**：assistant 文本增量 append；Markdown 对 **稳定前缀** 缓存（StreamingMarkdown 思路）。  
4. **One renderer, many surfaces**：`markdown_to_lines()` 同时用于 Chat assistant、Detail digest、Dashboard 选中项。  
5. **Performance budget**：Chat 300 行历史 redraw <16ms 典型终端宽 120；达不到则上虚拟滚动。  
6. **Keyboard contract 文档化**：快捷键写入 TUI hint + README；Chat 输入模式与全局模式分离（已做，需补 `/` 命令）。  

---

## 5. 分域改进计划

### 5.1 渲染引擎层（ratatui 基础设施）

| 项 | 描述 | 优先级 |
|----|------|--------|
| **全局背景 + Clear** | 每帧铺 `BG`，避免残影 | P0 ✅ 已做 |
| **Alternate screen** | 启动 TUI 进 alt-screen，退出还原 | P1 |
| **Resize 响应** | `Event::Resize` 触发 relayout | P1 |
| **Mouse（可选）** | 点击 Tab、滚动条、复制选区 | P3 |
| **主题配置** | `coworker.yaml` → `tui.theme: dark\|light\|none` | P2 |
| **帧率 / 动画时钟** | 统一 `Instant` 驱动 blink/spinner，避免多处 `SLOW_BLINK` | P2 |

**不建议**：引入 Ink/React；成本与 Rust 单 binary 目标冲突。

### 5.2 Chat 体验（对标 Claude Code REPL）

| 项 | 描述 | 优先级 |
|----|------|--------|
| **流式 assistant** | 新增 `AppEvent::ChatTokenDelta` / `ChatAssistantPartial`；engine 边收 Ollama chunk 边 push | P0 |
| **StreamingMarkdown** | `markdown.rs` 增加 `streaming_append(delta)` + stable prefix cache | P0 |
| **Turn 状态机** | `idle → thinking → tool(name) → streaming → idle`；footer 显示当前阶段 | P1 |
| **Esc 取消 turn** | 取消 LLM 请求 + MCP in-flight（需 `CancellationToken`） | P1 |
| **Tool 卡片** | 可折叠块：header 一行，展开看 cap 结果 | P1 |
| **重复 tool 警告** | Harness 检测相同 `(tool, args)` 失败；UI 显示 amber banner | P0 |
| **输入增强** | 多行 `Shift+Enter`；`↑` 历史；`/clear` `/help` | P2 |
| **Session 切换** | TUI 内列出 `chat_sessions`，切换 hydrate | P2 |
| **虚拟滚动** | 消息级而非行级：每条 message 缓存 `Vec<Line>` + 高度 | P1 |

#### 流式架构草图

```
Ollama stream chunk
    → llm/client.rs (chat stream API)
    → engine/chat.rs
    → AppEvent::ChatPartial { session_id, delta }
    → tui/chat.rs append to staging buffer
    → streaming_markdown.render()
    → ratatui draw (16ms throttle optional)
```

与 Claude Code 差异：我们中间还有 **JSON action 步**（tool 轮），需在 UI 上区分 **「模型在写 JSON」** vs **「模型在写 reply message」**。

### 5.3 Markdown 渲染（对标 StreamingMarkdown + cli-markdown）

| 项 | 描述 | 优先级 |
|----|------|--------|
| **Streaming 增量** | 稳定前缀 + 尾部 re-parse（见 §5.2） | P0 |
| **表格** | pulldown-cmark table → ratatui 简表（列宽按 terminal 分配） | P2 |
| **嵌套列表** | 正确 `list_depth` + 序号/ bullet 对齐 | P1 |
| **PR/Issue 智能链** | `#19235`、`<owner/repo>`、`run_id` 高亮（部分已有） | P1 |
| **链接** | 显示 `[title]` + 短链；OSC 8 真超链（支持则点击） | P2 |
| **代码块** | 语法高亮可选：`syntect` 或简单 keyword tint | P3 |
| **Digest 专用** | `## Needs attention` 等 section 标题统一层级 | P1 |

**避免**：把 `\n` 全转成 `\n\n`（旧 bug）；保持块级与 inline 分离。

### 5.4 Tool 透明度（对标 ToolUseLoader + progress stream）

| 项 | 描述 | 优先级 |
|----|------|--------|
| **参数摘要** | `pr_get_overview(repo, pr=19235)` 高亮缺失参数 | P0 |
| **耗时 + 状态色** | ✓ 绿 / ✗ 红 / ◔ 黄 blink | P0 ✅ 部分 |
| **Progress 流** | 长 MCP 操作（`ci_get_failed_logs` 分页）中间态 | P2 |
| **Tool 分组** | 同一 turn 内多个 tool 缩进一组 | P1 |
| **并行只读 tool** | Harness 允许 `join!` 两个只读 call；UI 并排状态 | P3 |

### 5.5 多 Tab 工作流 UI（OpenCode 侧栏 + 我们三栏 design）

| Tab | 改进 | 优先级 |
|-----|------|--------|
| **Dashboard** | Digest 摘要 Markdown 渲染；alert 卡片化 | P1 |
| **PRs** | CI/review 图标列；选中 → Detail 用 overview 格式 | P1 |
| **Approvals** | diff 式 before/after（tool args JSON pretty） | P2 |
| **Logs** | 级别色 + 时间轴；filter `/error` | P2 |
| **Config** | 连通性探针可视化（latency bar） | P3 |
| **Flaky** | sparkline 趋势（ASCII） | P3 |
| **Detail 面板** | 统一走 `markdown_to_lines` + 独立 scroll | P1 |

### 5.6 交互与辅助

| 项 | 描述 | 优先级 |
|----|------|--------|
| **Hint 栏 contextual** | 每个 Tab 一行；Chat 显示 scroll 键 | P1 ✅ 部分 |
| **Approval 防误触** | `y` 前 200ms 延迟或二次确认 mutating | P2 |
| **Cost / token 指示** | footer 显示本轮 chat input/output tokens（本地 Ollama 若可拿） | P3 |
| **Transcript 导出** | Chat 导出 Markdown（Store 已有 session） | P2 |
| **Bell / notify** | workflow 完成终端 bell（headless 友好） | P3 |

---

## 6. 分阶段路线图

### Phase A — 「能信」：透明 + 不傻 loop（1–2 周）

**目标**：用户能在 Chat 里看清 agent 在干什么，重复失败 tool 被 Harness 拦住。

| # | 任务 | 文件/模块 |
|---|------|-----------|
| A1 | Harness：相同 tool+args 连续失败 ≥2 → 强制 reply + 注入错误 hint | `chat_loop.rs` |
| A2 | Harness：`latest PR` 无 `#N` 时自动 `pr_list_open` → `pr_get_overview` | `chat_loop.rs` |
| A3 | UI：tool 行显示完整 args；失败行 amber/red | `theme.rs` |
| A4 | UI：Detail 面板 Markdown 渲染 digest body | `mod.rs`, `markdown.rs` |
| A5 | 测试：tool dedup、markdown digest snapshot tests | `tui/*`, `chat_loop` tests |

**验收**：复现「Analyze latest PR」不再连打 6 次 overview；Dashboard digest 可读。

> **状态 2026-06-12**：A1 ✅ · A2 ✅（`bootstrap_latest_pr_chain`）· A3 ✅ 部分 · A4 ✅ · A5 ✅ 部分

### Phase B — 「能看」：流式 Chat + StreamingMarkdown（2–3 周）

**目标**：assistant 回复像 Claude Code 一样增量出现。

| # | 任务 | 文件/模块 |
|---|------|-----------|
| B1 | Ollama chat **stream:true** + JSON 增量解析（或先 stream message 字段） | `llm/client.rs`, `llm/chat.rs` |
| B2 | `AppEvent::ChatPartial` + staging buffer in `AppState` | `app/mod.rs`, `engine/chat.rs` |
| B3 | `StreamingMarkdownRenderer` stable prefix | `tui/markdown.rs` |
| B4 | Turn 状态 footer + Esc cancel (`CancellationToken`) | `tui/chat.rs`, `engine/chat.rs` |
| B5 | 消息级虚拟滚动（缓存每条 message 的 `Vec<Line>`） | `tui/chat.rs` |

**验收**：长 PR 列表回复边生成边显示；滚动不卡顿；Esc 可取消 busy turn。

> **状态 2026-06-12**：B1–B3 ✅ · B4 ✅ 部分 · B5 ✅（消息级 viewport 虚拟滚动 + 高度缓存）

### Phase C — 「好用」：信息架构 + 工作流 Tab（2–3 周）

**目标**：非 Chat Tab 达到「日常运维面板」而非 debug 列表。

| # | 任务 |
|---|------|
| C1 | PR 列表：状态 glyph + 排序/filter UI |
| C2 | Dashboard：digest section 折叠 + markdown |
| C3 | Tool 结果折叠/展开（Chat + Detail） |
| C4 | Logs 过滤 + 颜色 |
| C5 | `tui.theme` 可配置 + resize 处理 |
| C6 | Chat：`/clear`、输入历史、multiline |

> **状态 2026-06-12**：C1–C6 ✅ · Detail 独立滚动 `{`/`}` · Chat `/sessions` `/session` `/export` · Approvals detail 增强

### Phase D — 「可选增强」（按需）

- 表格 Markdown、OSC 8 链接、syntect 代码高亮  
- 并行只读 MCP tool  
- Mouse 支持、侧栏 token 预算（OpenCode 风）  
- TUI attach 模式下 Chat 与 daemon 同步会话  

---

## 7. 技术决策记录

| 决策 | 选项 | 结论 |
|------|------|------|
| TUI 框架 | ratatui vs Ink/React | **保持 ratatui**；秘书场景不需要 Flexbox 50 组件 |
| Markdown | pulldown-cmark vs comrak vs external pager | **pulldown-cmark** + 自研 Streaming 层 |
| 流式 LLM | 等 turn 结束 vs SSE | **Phase B 起 stream**；tool 步仍逐步 |
| Chat 布局 | 全屏单栏 vs 左聊右 context | **暂单栏**；Phase D 可加右栏 Store/digest 预览 |
| 状态管理 | 纯 `AppState` vs Elm Msg | **维持 AppState + apply_event**；复杂后可抽 `tui/state.rs` |
| 测试 | 截图 vs 单元 | **Line snapshot tests**（已有 markdown/theme tests） |

---

## 8. 非目标（明确不做）

- 行级 code diff 编辑 UI（交给 IDE / `gh`）  
- 完整 Vim 模式 / 鼠标选中复制（除非 Phase D 有需求）  
- Web Dashboard 或 Electron 壳  
- 为 TUI 引入 Node.js 或 bundler  
- 与 Claude Code 1:1 功能对等（权限 classifier shimmer、180 语言 diff 等）  

---

## 9. 参考

| 来源 | 链接 |
|------|------|
| Claude Code UX 剖析 | [how-claude-code-works Ch.14](https://github.com/Windy3f3f3f3f/how-claude-code-works/blob/main/en/docs/12-user-experience.md) |
| Claude Code 架构总览 | [openedclaude architecture](https://github.com/openedclaude/claude-reviews-claude/blob/main/architecture/00-overview.md) |
| OpenCode TUI 观察 | [TUICommander — OpenCode](https://tuicommander.com/docs/architecture/agents/opencode.html) |
| OpenCode vs Aider | [MorphLLM comparison](https://www.morphllm.com/comparisons/opencode-vs-aider) |
| 本项目设计 | `design.md` § TUI、`README.md` § TUI |
| 近期 Chat/TUI 实现 | `src/tui/*`, `src/agent/chat_loop.rs`, `ChatProgress` |

---

## 10. 下一步建议

若只选 **一个** 高 ROI 迭代：**Phase A（Harness 防 loop + Detail Markdown）**，成本低、直接解决已暴露的信任问题。

若要做 **体感最大** 的升级：紧接着 **Phase B 流式 Chat**，这是对 Claude Code 差距最大、用户最能感知的一项。

---

*文档版本：2026-06-12 · 对应当前 `main` @ 62773d0 之后本地 TUI 美化改动。*
