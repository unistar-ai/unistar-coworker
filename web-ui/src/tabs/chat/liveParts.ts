import { buildChatBlocks } from "./parser";
import type { ChatBlock } from "./parser";
import {
  blocksToProcessParts,
  partsProcessStats,
  type ChatMessagePart,
  type ReasoningPart,
  type ToolPart,
} from "./messageParts";
import { formatTurnProcessSummary } from "./parser";
import type { DisplayStep } from "./partDisplay";

export interface LiveTransportState {
  streaming: string | null;
  reasoning: string | null;
  toolRunning: string | null;
  toolRunningDetail: string | null;
  toolPending: string | null;
  compressing: boolean;
  activityFlow: { kind: string; text: string } | null;
  turnPhase: string | null;
}

function isUserBlock(block: ChatBlock): boolean {
  return block.type === "message" && block.message?.role === "you";
}

function isProcessBlock(block: ChatBlock): boolean {
  return (
    block.type === "reasoning" ||
    block.type === "tool-group" ||
    block.type === "tool-batch"
  );
}

/** Process blocks for the in-flight agent turn (since last user message). */
export function extractInProgressProcessBlocks(
  lines: string[],
  outputs: Record<string, string>,
  originals: Record<string, string> = {},
): ChatBlock[] {
  const blocks = buildChatBlocks(lines, outputs, originals);
  let start = 0;
  for (let i = blocks.length - 1; i >= 0; i--) {
    if (isUserBlock(blocks[i])) {
      start = i + 1;
      break;
    }
  }
  const process: ChatBlock[] = [];
  for (const block of blocks.slice(start)) {
    if (isProcessBlock(block)) {
      process.push(block);
    } else if (block.type === "message" && block.message?.role === "assistant") {
      process.push(block);
    }
  }
  return process;
}

/** Overlay WS live fields onto committed process parts. */
export function applyLiveTransportToParts(
  processParts: ChatMessagePart[],
  live: LiveTransportState,
): ChatMessagePart[] {
  const parts = [...processParts];

  if (live.reasoning && !live.streaming) {
    let lastReasonIdx = -1;
    for (let i = parts.length - 1; i >= 0; i--) {
      if (parts[i].kind === "reasoning") {
        lastReasonIdx = i;
        break;
      }
    }
    if (lastReasonIdx >= 0) {
      const prev = parts[lastReasonIdx] as ReasoningPart;
      parts[lastReasonIdx] = { ...prev, text: live.reasoning };
    } else {
      parts.push({ id: "live-reasoning", kind: "reasoning", text: live.reasoning });
    }
  }

  const toolName = live.toolRunning || live.toolPending;
  if (toolName) {
    let matchIdx = -1;
    for (let i = parts.length - 1; i >= 0; i--) {
      const p = parts[i];
      if (p.kind === "tool" && p.group.toolName === toolName) {
        matchIdx = i;
        break;
      }
    }
    const status = live.toolRunning ? "running" : "pending";
    // `toolRunningDetail` is progress/elapsed (e.g. "23s"), NOT tool args.
    if (matchIdx >= 0) {
      const prev = parts[matchIdx] as ToolPart;
      parts[matchIdx] = {
        ...prev,
        group: {
          ...prev.group,
          status,
        },
      };
    } else {
      parts.push({
        id: `live-tool-${toolName}`,
        kind: "tool",
        blockKey: `live-tool-${toolName}`,
        group: {
          toolName,
          status,
          ms: null,
          args: null,
          steps: [],
        },
      });
    }
  }

  return parts;
}

export function buildLiveAgentProcessParts(
  lines: string[],
  outputs: Record<string, string>,
  originals: Record<string, string>,
  live: LiveTransportState,
): ChatMessagePart[] {
  const blocks = extractInProgressProcessBlocks(lines, outputs, originals);
  const committed = blocksToProcessParts(blocks);
  return applyLiveTransportToParts(committed, live);
}

/** Prefer WS `chat_turn_parts`; fall back to parsing `chat_lines`.
 *  When WS tool groups lack steps/outputs, merge from the line-parser path. */
export function resolveLiveProcessParts(
  wsParts: ChatMessagePart[] | null | undefined,
  lines: string[],
  outputs: Record<string, string>,
  originals: Record<string, string>,
  live: LiveTransportState,
): ChatMessagePart[] {
  const fromLines = blocksToProcessParts(
    extractInProgressProcessBlocks(lines, outputs, originals),
  );
  const base =
    wsParts && wsParts.length > 0
      ? mergeWsPartsWithLineParts(wsParts, fromLines)
      : fromLines;
  return applyLiveTransportToParts(base, live);
}

/** Fill empty tool `steps` from the richer line-parser parts (same tool order). */
export function mergeWsPartsWithLineParts(
  wsParts: ChatMessagePart[],
  lineParts: ChatMessagePart[],
): ChatMessagePart[] {
  const lineTools = lineParts.filter((p): p is ToolPart => p.kind === "tool");
  let toolIdx = 0;
  return wsParts.map((part) => {
    if (part.kind !== "tool") return part;
    const fallback = lineTools[toolIdx++];
    const steps = part.group.steps || [];
    const hasOutput = steps.some((s) => s.output);
    if (steps.length > 0 && hasOutput) return part;
    if (!fallback) return part;
    return {
      ...part,
      group: {
        ...part.group,
        steps: fallback.group.steps?.length ? fallback.group.steps : part.group.steps,
        args: part.group.args ?? fallback.group.args,
        ms: part.group.ms ?? fallback.group.ms,
        status: part.group.status || fallback.group.status,
      },
    };
  });
}

/** Part-level + transport-level "process still running" (Cherry part hold). */
export function isLiveProcessActive(
  parts: ChatMessagePart[],
  live: Pick<
    LiveTransportState,
    | "reasoning"
    | "streaming"
    | "toolRunning"
    | "toolPending"
    | "compressing"
    | "activityFlow"
  >,
): boolean {
  if (live.toolRunning || live.toolPending) return true;
  if (live.reasoning && !live.streaming) return true;
  if (live.compressing || live.activityFlow) return true;
  for (let i = parts.length - 1; i >= 0; i--) {
    const p = parts[i];
    if (p.kind === "tool") {
      const st = p.group.status;
      return st === "running" || st === "pending";
    }
    if (p.kind === "reasoning" || p.kind === "tool-batch") break;
  }
  return false;
}

export function liveProcessSummary(
  parts: ChatMessagePart[],
  live: LiveTransportState,
  elapsedMs?: number | null,
): string {
  const stats = partsProcessStats(parts);
  const fromStats = formatTurnProcessSummary(stats, elapsedMs);
  if (fromStats) return fromStats;
  if (live.toolRunning || live.toolPending) return "工具执行中";
  if (live.reasoning && !live.streaming) return "深度思考中";
  if (live.compressing) return "压缩上下文中";
  if (live.activityFlow) return "活动进行中";
  return "处理中";
}

/** Mark in-flight reasoning/tool rows for live styling. */
export function decorateLiveDisplaySteps(
  steps: DisplayStep[],
  live: Pick<
    LiveTransportState,
    "reasoning" | "streaming" | "toolRunning" | "toolPending" | "toolRunningDetail"
  >,
): DisplayStep[] {
  const out = steps.map((s) => ({ ...s }));
  const toolName = live.toolRunning || live.toolPending;
  if (toolName) {
    for (let i = out.length - 1; i >= 0; i--) {
      if (out[i].kind === "tool" && out[i].toolName === toolName) {
        out[i] = {
          ...out[i],
          isLive: true,
          status: live.toolRunning
            ? live.toolRunningDetail?.trim() || "执行中"
            : "等待中",
          statusKind: live.toolRunning ? "running" : "pending",
        };
        break;
      }
    }
  }
  if (live.reasoning && !live.streaming) {
    for (let i = out.length - 1; i >= 0; i--) {
      if (out[i].kind === "thought") {
        out[i] = {
          ...out[i],
          isLive: true,
          liveDetailBody: live.reasoning,
          title: "思考中",
          subtitle: live.reasoning.replace(/\s+/g, " ").trim().slice(0, 100) || out[i].subtitle,
          status: undefined,
          statusKind: "running",
        };
        break;
      }
    }
  }
  return out;
}

export function liveHasProcessPanel(
  parts: ChatMessagePart[],
  live: LiveTransportState,
): boolean {
  if (parts.length > 0) return true;
  return Boolean(
    live.toolRunning ||
      live.toolPending ||
      live.activityFlow ||
      live.compressing ||
      (live.reasoning && !live.streaming),
  );
}
