// Chat transcript parser — mirrors legacy app.js::parseMessage / parseToolStep,
// extended with: reasoning-as-block, interim assistant in tool runs, tool
// source labels, and tool arg chips.

export interface ChatMessage {
  role: "you" | "assistant" | "system" | "error" | "tool" | "meta";
  badge: string;
  body: string;
  lineIndex: number;
  md: boolean;
}

export type ToolStepKind =
  | "start"
  | "done"
  | "approval-pending"
  | "approval"
  | "warn"
  | "reasoning"
  | "interim"
  | "meta";

export interface ToolStep {
  kind: ToolStepKind;
  text: string;
  index: number;
  name?: string;
  args?: string | null;
  ok?: boolean | null;
  ms?: string | null;
  output?: string | null;
}

export interface ToolGroup {
  steps: ToolStep[];
  toolName: string;
  status: "ok" | "err" | "pending" | "running" | "warn" | "neutral";
  ms: string | null;
  args: string | null;
}

export type ChatBlockType = "message" | "tool-group" | "tool-batch" | "reasoning" | "meta";

export interface ChatBlock {
  type: ChatBlockType;
  message?: ChatMessage;
  /** Single tool group (1–2 tools, or expanded from batch). */
  group?: ToolGroup;
  steps?: ToolStep[];
  groups?: ToolGroup[];
  batchId?: string;
  /** For reasoning blocks: the normalized reasoning text. */
  reasoningText?: string;
  /** For reasoning blocks: the raw line text (before normalization). */
  reasoningRaw?: string;
  /** True on the last assistant message block — used for Regenerate button. */
  isLastAssistant?: boolean;
  key: string;
}

// --- Tool metadata (icon + label + source) — ported from legacy ---

const TOOL_META: Record<string, { icon: string; label: string }> = {
  bash_run: { icon: "⌘", label: "Bash" },
  python_run: { icon: "🐍", label: "Python" },
  web_fetch: { icon: "🌐", label: "Fetch" },
  web_browser: { icon: "🌐", label: "Fetch" },
  read_file: { icon: "📄", label: "Read" },
  write_file: { icon: "✎", label: "Write" },
  edit_file: { icon: "✎", label: "Edit" },
  grep: { icon: "🔍", label: "Grep" },
  glob: { icon: "📁", label: "Glob" },
  skill_search: { icon: "📚", label: "Skill search" },
  skill_load: { icon: "📚", label: "Load skill" },
  tool_search: { icon: "🔎", label: "Tool search" },
  tool_call: { icon: "⚡", label: "Tool call" },
  pr_get_diff: { icon: "⎇", label: "PR diff" },
  pr_get_overview: { icon: "◫", label: "PR overview" },
  pr_list_changed_files: { icon: "📋", label: "Changed files" },
  pr_diff_risk_scan: { icon: "⚠", label: "Diff risk" },
  pr_get_ci_snapshot: { icon: "◎", label: "CI snapshot" },
  pr_get_review_routing: { icon: "👥", label: "Review routing" },
  pr_get_review_state: { icon: "✓", label: "Review state" },
};

export interface ToolSource {
  source: string;
  detail: string;
}

/** Resolve the backend source of a tool name (github / mcp:<id> / local). */
export function toolSourceLabel(
  toolName: string | undefined,
  mcpServers: { id: string; prefix: string }[] = [],
): ToolSource | null {
  if (!toolName) return null;
  for (const s of mcpServers) {
    const prefix = s.prefix || `${s.id}_`;
    if (toolName.startsWith(prefix)) {
      return { source: `mcp:${s.id}`, detail: toolName.slice(prefix.length) || toolName };
    }
  }
  if (/^(pr_|ci_|issue_|repo_|release_|notify_)/.test(toolName)) {
    return { source: "github", detail: toolName };
  }
  if (
    [
      "bash_run",
      "python_run",
      "read_file",
      "write_file",
      "edit_file",
      "grep",
      "glob",
      "web_fetch",
      "web_browser",
      "skill_load",
      "skill_search",
    ].includes(toolName)
  ) {
    return { source: "local", detail: toolName };
  }
  return null;
}

export interface ToolMeta {
  icon: string;
  label: string;
  source?: ToolSource;
}

export function toolMeta(
  name: string | undefined,
  mcpServers: { id: string; prefix: string }[] = [],
): ToolMeta {
  const key = (name || "").toLowerCase();
  const base = TOOL_META[key] || { icon: "⚙", label: name || "tool" };
  const source = toolSourceLabel(name, mcpServers);
  return source ? { ...base, source } : base;
}

// --- Tool arg chips — ported from legacy ---

export interface ArgPair {
  key: string;
  value: string;
}

export function parseToolArgsString(args: string | null | undefined): ArgPair[] {
  if (!args?.trim()) return [];
  const out: ArgPair[] = [];
  for (const part of args.split(",")) {
    const t = part.trim();
    if (!t) continue;
    const eq = t.indexOf("=");
    if (eq > 0) {
      out.push({ key: t.slice(0, eq).trim(), value: t.slice(eq + 1).trim() });
    } else {
      out.push({ key: t, value: "" });
    }
  }
  return out;
}

function truncateMiddle(s: string, max: number): string {
  if (s.length <= max) return s;
  const half = Math.floor((max - 1) / 2);
  return s.slice(0, half) + "…" + s.slice(s.length - half);
}

export function formatToolArgValue(key: string, value: string): string {
  const k = key.toLowerCase();
  if (!value) return "";
  if (k === "pr_number" || k === "pr") return `#${value}`;
  if (k === "max_bytes") {
    const n = Number.parseInt(value, 10);
    if (Number.isFinite(n) && n >= 1000) return `${Math.round(n / 1000)}k`;
    return value;
  }
  if (k === "repo") return truncateMiddle(value, 28);
  return truncateMiddle(value, 20);
}

// --- Block summarization (mirrors legacy) ---

/** Summarize a group of tool steps into a single status. */
export function summarizeToolGroup(steps: ToolStep[]): ToolGroup {
  const named = steps.find((s) => s.name);
  const toolName = named?.name || steps.find((s) => s.kind === "start")?.name || "tool";
  const done = [...steps].reverse().find((s) => s.kind === "done");
  const pending = steps.some((s) => s.kind === "approval-pending");
  let status: ToolGroup["status"] = "neutral";
  if (done) status = done.ok ? "ok" : "err";
  else if (pending) status = "pending";
  else if (steps.some((s) => s.kind === "start")) status = "running";
  else if (steps.some((s) => s.kind === "warn")) status = "warn";
  const ms = done?.ms || null;
  const args = done?.args || steps.find((s) => s.args)?.args || null;
  return { steps, toolName, status, ms, args };
}

/** Pair tool start/done rows — parallel tools interleave `→` before all `✓`. */
export function splitToolStepGroups(steps: ToolStep[]): ToolStep[][] {
  const pending: { steps: ToolStep[] }[] = [];
  const groups: ToolStep[][] = [];

  const pushGroup = (stepList: ToolStep[]) => {
    if (stepList.length) groups.push(stepList);
  };

  const firstPendingStartIndex = (toolName: string | undefined): number => {
    if (!toolName) return -1;
    for (let i = 0; i < pending.length; i++) {
      const start = pending[i].steps.find((s) => s.kind === "start");
      if (start?.name === toolName) return i;
    }
    return -1;
  };

  for (const step of steps) {
    if (step.kind === "start") {
      pending.push({ steps: [step] });
      continue;
    }
    if (step.kind === "done") {
      const matchIdx = firstPendingStartIndex(step.name);
      if (matchIdx >= 0) {
        const group = pending.splice(matchIdx, 1)[0];
        group.steps.push(step);
        pushGroup(group.steps);
      } else {
        pushGroup([step]);
      }
      continue;
    }
    if (step.kind === "approval") {
      const matchIdx = firstPendingStartIndex(step.name);
      if (matchIdx >= 0) {
        pending[matchIdx].steps.push(step);
      } else if (pending.length) {
        pending[pending.length - 1].steps.push(step);
      } else {
        pushGroup([step]);
      }
      continue;
    }
    // interim / reasoning / warn / approval-pending / meta — attach to last pending or last group
    if (pending.length) {
      pending[pending.length - 1].steps.push(step);
    } else if (groups.length) {
      groups[groups.length - 1].push(step);
    } else {
      pushGroup([step]);
    }
  }

  for (const g of pending) {
    pushGroup(g.steps);
  }
  return groups;
}

// --- Line parsing ---

export function parseMessage(line: string, lineIndex: number): ChatMessage {
  if (line.startsWith("you> "))
    return { role: "you", badge: "You", body: line.slice(5), lineIndex, md: true };
  if (line.startsWith("assistant> "))
    return {
      role: "assistant",
      badge: "AI",
      body: line.slice(11),
      lineIndex,
      md: true,
    };
  if (line.startsWith("system> "))
    return { role: "system", badge: "system", body: line.slice(8), lineIndex, md: false };
  if (line.startsWith("error> "))
    return { role: "error", badge: "error", body: line.slice(7), lineIndex, md: false };
  if (line.startsWith("chat> "))
    return { role: "meta", badge: "chat", body: line.slice(6), lineIndex, md: false };
  if (line.startsWith("  ✓ "))
    return { role: "tool", badge: "✓", body: line.slice(4), lineIndex, md: false };
  if (line.startsWith("  → "))
    return { role: "tool", badge: "→", body: line.slice(4), lineIndex, md: false };
  if (line.startsWith("  ✗ "))
    return { role: "tool", badge: "✗", body: line.slice(4), lineIndex, md: false };
  if (line.startsWith("  ⚠ "))
    return { role: "tool", badge: "⚠", body: line.slice(4), lineIndex, md: false };
  if (line.startsWith("  ⏳ "))
    return { role: "tool", badge: "⏳", body: line.slice(4), lineIndex, md: false };
  if (line.startsWith("  … "))
    return { role: "tool", badge: "…", body: line.slice(4), lineIndex, md: false };
  return { role: "meta", badge: "·", body: line, lineIndex, md: false };
}

function splitToolCall(body: string): { name: string; args: string | null } {
  const m = body.match(/^([\w-]+)(?:\((.*)\))?$/);
  return { name: m?.[1] || body, args: m?.[2] || null };
}

function splitToolDone(body: string): { name: string; args: string | null; ms: string | null } {
  const msM = body.match(/\((\d+)ms\)\s*$/);
  const ms = msM ? msM[1] : null;
  const rest = msM ? body.slice(0, msM.index).trim() : body;
  const call = splitToolCall(rest);
  return { ...call, ms };
}

export function parseToolStep(
  line: string,
  index: number,
  outputs: Record<string, string>,
): ToolStep {
  const output = outputs[String(index)] ?? outputs[index] ?? null;
  if (line.startsWith("  → ")) {
    const body = line.slice(4);
    return { kind: "start", text: body, index, ...splitToolCall(body) };
  }
  if (line.startsWith("  ⏳ ")) {
    return { kind: "approval-pending", text: line.slice(4), index, ok: null };
  }
  if (line.startsWith("  ✓ ") || line.startsWith("  ✗ ")) {
    const ok = line.startsWith("  ✓ ");
    const body = line.slice(4);
    if (/^approval (resolved|approved|denied|failed)/i.test(body)) {
      return { kind: "approval", text: body, index, ok };
    }
    return { kind: "done", text: body, index, ok, output, ...splitToolDone(body) };
  }
  if (line.startsWith("  ⚠ ")) {
    return { kind: "warn", text: line.slice(4), index };
  }
  if (line.startsWith("  … ")) {
    return { kind: "reasoning", text: line.slice(4), index, output };
  }
  if (line.startsWith("chat> ")) {
    return { kind: "meta", text: line.slice(6), index };
  }
  const p = parseMessage(line, index);
  return { kind: "meta", text: p.body, index };
}

function isToolStepLine(line: string): boolean {
  return (
    line.startsWith("  → ") ||
    line.startsWith("  ✓ ") ||
    line.startsWith("  ✗ ") ||
    line.startsWith("  ⚠ ") ||
    line.startsWith("  ⏳ ") ||
    line.startsWith("  … ")
  );
}

function peekSignificantLine(lines: string[], fromIndex: number, direction: number): string | null {
  for (
    let i = fromIndex + direction;
    direction < 0 ? i >= 0 : i < lines.length;
    i += direction
  ) {
    const line = lines[i];
    if (!line.trim()) continue;
    return line;
  }
  return null;
}

/** Short assistant narration between tool calls in the same turn (not the final reply). */
function isInterimAssistantInToolRun(lines: string[], index: number): boolean {
  const line = lines[index];
  if (!line.startsWith("assistant> ")) return false;
  const body = line.slice(11).trim();
  if (!body || body.length > 800 || body.startsWith("{")) return false;
  if (/^tool_result\(/i.test(body)) return false;
  const prev = peekSignificantLine(lines, index, -1);
  if (!prev || !isToolTranscriptLine(prev)) return false;
  const next = peekSignificantLine(lines, index, 1);
  if (!next) return false;
  if (next.startsWith("you> ") || next.startsWith("error> ")) return false;
  return isToolTranscriptLine(next);
}

function isToolTranscriptLine(line: string): boolean {
  return isToolStepLine(line) || line.startsWith("chat> ");
}

/** Normalize reasoning text (strip "[agent reasoning summary]" prefix etc.). */
export function normalizeReasoningText(text: string | null | undefined): string {
  if (!text) return "";
  let s = String(text).trim();
  s = s.replace(/^\[agent reasoning summary\]\s*/i, "");
  s = s.replace(/^reasoning:\s*/i, "");
  return s.trim();
}

function getToolOutput(index: number, outputs: Record<string, string>): string | null {
  return outputs[String(index)] ?? outputs[index] ?? null;
}

function isPrimaryBlock(parsed: ChatMessage): boolean {
  return parsed.role === "you" || parsed.role === "assistant" || parsed.role === "error";
}

type RawBlock =
  | { type: "message"; message: ChatMessage; index: number }
  | {
      type: "reasoning";
      reasoningText: string;
      index: number;
    }
  | {
      type: "tool-group";
      steps: ToolStep[];
      index: number;
      toolName: string;
      status: ToolGroup["status"];
      ms: string | null;
      args: string | null;
    };

function pushToolStepBlocks(blocks: RawBlock[], steps: ToolStep[], outputs: Record<string, string>) {
  if (!steps.length) return;
  if (steps.every((s) => s.kind === "reasoning")) {
    const fullText = steps
      .map((s) => getToolOutput(s.index, outputs) || s.text)
      .filter(Boolean)
      .map(normalizeReasoningText)
      .join("\n\n");
    if (fullText) {
      blocks.push({
        type: "reasoning",
        reasoningText: fullText,
        index: steps[0].index,
      });
    }
    return;
  }
  if (steps.length === 1 && steps[0].kind === "meta") {
    blocks.push({
      type: "message",
      message: {
        role: "system",
        badge: "system",
        body: steps[0].text,
        lineIndex: steps[0].index,
        md: false,
      },
      index: steps[0].index,
    });
    return;
  }
  for (const groupSteps of splitToolStepGroups(steps)) {
    if (groupSteps.every((s) => s.kind === "reasoning")) {
      const fullText = groupSteps
        .map((s) => getToolOutput(s.index, outputs) || s.text)
        .filter(Boolean)
        .map(normalizeReasoningText)
        .join("\n\n");
      if (fullText) {
        blocks.push({
          type: "reasoning",
          reasoningText: fullText,
          index: groupSteps[0].index,
        });
      }
    } else if (groupSteps.length === 1 && groupSteps[0].kind === "meta") {
      blocks.push({
        type: "message",
        message: {
          role: "system",
          badge: "system",
          body: groupSteps[0].text,
          lineIndex: groupSteps[0].index,
          md: false,
        },
        index: groupSteps[0].index,
      });
    } else {
      const summarized = summarizeToolGroup(groupSteps);
      blocks.push({
        type: "tool-group",
        index: groupSteps[0].index,
        ...summarized,
      });
    }
  }
}

/** Mirrors legacy app.js::buildMessageBlocks — one tool run per user turn. */
function buildMessageBlocks(lines: string[], outputs: Record<string, string>): RawBlock[] {
  const blocks: RawBlock[] = [];
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    if (!line.trim()) {
      i++;
      continue;
    }
    const parsed = parseMessage(line, i);
    if (isPrimaryBlock(parsed)) {
      blocks.push({ type: "message", message: parsed, index: i });
      i++;
      continue;
    }
    if (line.startsWith("chat> ")) {
      blocks.push({
        type: "message",
        message: {
          role: "system",
          badge: "chat",
          body: line.slice(6),
          lineIndex: i,
          md: false,
        },
        index: i,
      });
      i++;
      continue;
    }
    const steps: ToolStep[] = [];
    while (i < lines.length) {
      const l = lines[i];
      if (!l.trim()) {
        i++;
        continue;
      }
      if (l.startsWith("you> ") || l.startsWith("error> ")) break;
      if (l.startsWith("assistant> ")) {
        if (isInterimAssistantInToolRun(lines, i)) {
          steps.push({ kind: "interim", text: l.slice(11).trim(), index: i });
          i++;
          continue;
        }
        break;
      }
      if (l.startsWith("chat> ")) break;
      if (l.startsWith("system> ")) {
        if (steps.length) {
          pushToolStepBlocks(blocks, steps, outputs);
          steps.length = 0;
        }
        blocks.push({
          type: "message",
          message: {
            role: "system",
            badge: "system",
            body: l.slice(8),
            lineIndex: i,
            md: false,
          },
          index: i,
        });
        i++;
        continue;
      }
      steps.push(parseToolStep(l, i, outputs));
      i++;
    }
    pushToolStepBlocks(blocks, steps, outputs);
  }
  return blocks;
}

/** Merge 3+ consecutive completed tool groups into one compact strip (legacy). */
function mergeConsecutiveToolGroups(blocks: RawBlock[]): ChatBlock[] {
  const out: ChatBlock[] = [];
  let run: Extract<RawBlock, { type: "tool-group" }>[] = [];
  let key = 0;

  const flush = () => {
    if (!run.length) return;
    const batchId = `tb-${run[0].index ?? 0}-${run.length}`;
    if (run.length >= 3) {
      const groups = run.map((g) => summarizeToolGroup(g.steps));
      out.push({
        type: "tool-batch",
        groups,
        steps: run.flatMap((g) => g.steps),
        batchId,
        key: batchId,
      });
    } else {
      for (const g of run) {
        out.push({
          type: "tool-group",
          group: summarizeToolGroup(g.steps),
          steps: g.steps,
          key: `tg-${g.index}-${key++}`,
        });
      }
    }
    run = [];
  };

  for (const b of blocks) {
    const batchable =
      b.type === "tool-group" &&
      (b.status === "ok" || b.status === "err" || b.status === "warn" || b.status === "neutral");
    if (batchable) {
      run.push(b);
    } else {
      flush();
      if (b.type === "message") {
        out.push({ type: "message", message: b.message, key: `m-${b.index}-${key++}` });
      } else if (b.type === "reasoning") {
        out.push({
          type: "reasoning",
          reasoningText: b.reasoningText,
          key: `r-${b.index}-${key++}`,
        });
      } else if (b.type === "tool-group") {
        out.push({
          type: "tool-group",
          group: summarizeToolGroup(b.steps),
          steps: b.steps,
          key: `tg-${b.index}-${key++}`,
        });
      }
    }
  }
  flush();
  return out;
}

/** Group chat_lines into blocks — mirrors legacy buildMessageBlocks + mergeConsecutiveToolGroups. */
export function buildChatBlocks(
  lines: string[],
  outputs: Record<string, string>,
): ChatBlock[] {
  return mergeConsecutiveToolGroups(buildMessageBlocks(lines, outputs));
}

export interface MessageStats {
  blocks: number;
  you: number;
  ai: number;
  tools: number;
  reasoning: number;
}

/** Block counts for the messages header (legacy messageStatsFromLines). */
export function messageStatsFromBlocks(blocks: ChatBlock[]): MessageStats {
  const toolCount =
    blocks.filter((b) => b.type === "tool-group").length +
    blocks.filter((b) => b.type === "tool-batch").reduce(
      (n, b) => n + (b.groups?.length || 0),
      0,
    );
  return {
    blocks: blocks.length,
    you: blocks.filter((b) => b.type === "message" && b.message?.role === "you")
      .length,
    ai: blocks.filter(
      (b) => b.type === "message" && b.message?.role === "assistant",
    ).length,
    tools: toolCount,
    reasoning: blocks.filter((b) => b.type === "reasoning").length,
  };
}

export function formatMessageCount(stats: MessageStats): string {
  if (!stats.blocks) return "";
  const parts = [`${stats.blocks} blocks`];
  if (stats.you) parts.push(`${stats.you} you`);
  if (stats.ai) parts.push(`${stats.ai} ai`);
  if (stats.tools) parts.push(`${stats.tools} tools`);
  return parts.join(" · ");
}

export function formatTokens(n: number): string {
  if (n >= 10000) return (n / 1000).toFixed(1) + "k";
  if (n >= 1000) return (n / 1000).toFixed(2) + "k";
  return String(n);
}
