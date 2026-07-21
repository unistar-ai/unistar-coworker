import type { AgentTurn, ChatBlock, ToolGroup } from "./parser";

/** Frontend part protocol — adapter skeleton for future backend parts streaming. */
export type ChatMessagePartKind = "text" | "reasoning" | "tool" | "tool-batch";

export interface ChatMessagePartBase {
  id: string;
  kind: ChatMessagePartKind;
}

export interface TextPart extends ChatMessagePartBase {
  kind: "text";
  role: "user" | "assistant";
  text: string;
  md?: boolean;
  /** True on the final assistant answer block in a turn. */
  isAnswer?: boolean;
}

export interface ReasoningPart extends ChatMessagePartBase {
  kind: "reasoning";
  text: string;
  original?: string;
}

export interface ToolPart extends ChatMessagePartBase {
  kind: "tool";
  group: ToolGroup;
  blockKey: string;
}

export interface ToolBatchPart extends ChatMessagePartBase {
  kind: "tool-batch";
  groups: ToolGroup[];
  blockKey: string;
}

export type ChatMessagePart = TextPart | ReasoningPart | ToolPart | ToolBatchPart;

export function isProcessPart(part: ChatMessagePart): boolean {
  if (part.kind === "reasoning" || part.kind === "tool" || part.kind === "tool-batch") {
    return true;
  }
  return part.kind === "text" && part.role === "assistant" && !part.isAnswer;
}

export interface TurnPartsSplit {
  processParts: ChatMessagePart[];
  answerParts: ChatMessagePart[];
}

/** Split agent parts into collapsed process vs trailing answer (Cherry `getToolHistoryGroup`). */
export function splitTurnParts(parts: ChatMessagePart[]): TurnPartsSplit {
  const answerStart = parts.findIndex((p) => p.kind === "text" && p.isAnswer);
  if (answerStart >= 0) {
    return {
      processParts: parts.slice(0, answerStart),
      answerParts: parts.slice(answerStart),
    };
  }
  let lastProcess = -1;
  for (let i = 0; i < parts.length; i++) {
    if (isProcessPart(parts[i])) lastProcess = i;
  }
  if (lastProcess < 0) return { processParts: [], answerParts: parts };
  return {
    processParts: parts.slice(0, lastProcess + 1),
    answerParts: parts.slice(lastProcess + 1),
  };
}

export function partsProcessStats(parts: ChatMessagePart[]): { tools: number; thoughts: number } {
  let tools = 0;
  let thoughts = 0;
  for (const part of parts) {
    if (part.kind === "reasoning") thoughts++;
    else if (part.kind === "tool") tools++;
    else if (part.kind === "tool-batch") tools += part.groups.length;
  }
  return { tools, thoughts };
}

/** Agent-only parts (process + answer), excluding user message. */
export function turnAgentParts(turn: AgentTurn): ChatMessagePart[] {
  const parts: ChatMessagePart[] = [];
  for (const block of turn.process) parts.push(...blockToParts(block));
  if (turn.answer) {
    const answerParts = blockToParts(turn.answer);
    if (answerParts.length > 0) {
      const last = answerParts[answerParts.length - 1];
      if (last.kind === "text") last.isAnswer = true;
    }
    parts.push(...answerParts);
  }
  return parts;
}

/**
 * Prefer backend history process parts when present; always attach the
 * turn's answer from the line parser (answers are not in process parts).
 */
export function resolveHistoryAgentParts(
  turn: AgentTurn,
  historyPartsByUserLine: Record<string, ChatMessagePart[]> | null | undefined,
): ChatMessagePart[] {
  const userLine = turn.user?.message?.lineIndex;
  const wsProcess =
    userLine != null && historyPartsByUserLine
      ? historyPartsByUserLine[String(userLine)]
      : undefined;
  if (wsProcess && wsProcess.length > 0) {
    const parts: ChatMessagePart[] = [...wsProcess];
    if (turn.answer) {
      const answerParts = blockToParts(turn.answer);
      if (answerParts.length > 0) {
        const last = answerParts[answerParts.length - 1];
        if (last.kind === "text") last.isAnswer = true;
      }
      parts.push(...answerParts);
    }
    return parts;
  }
  return turnAgentParts(turn);
}

/** Blocks → parts (no turn wrapper). */
export function blocksToProcessParts(blocks: ChatBlock[]): ChatMessagePart[] {
  return blocks.flatMap((block) => blockToParts(block));
}

function blockToParts(block: ChatBlock): ChatMessagePart[] {
  if (block.type === "reasoning") {
    return [
      {
        id: block.key,
        kind: "reasoning",
        text: block.reasoningText || "",
        original: block.reasoningOriginal,
      },
    ];
  }
  if (block.type === "tool-group" && block.group) {
    return [{ id: block.key, kind: "tool", group: block.group, blockKey: block.key }];
  }
  if (block.type === "tool-batch") {
    return [
      {
        id: block.key,
        kind: "tool-batch",
        groups: block.groups || [],
        blockKey: block.key,
      },
    ];
  }
  if (block.type === "message" && block.message) {
    const role = block.message.role === "you" ? "user" : "assistant";
    return [
      {
        id: block.key,
        kind: "text",
        role,
        text: block.message.body,
        md: block.message.md,
      },
    ];
  }
  return [];
}

/** Convert an `AgentTurn` into ordered message parts (user → process → answer). */
export function turnBlocksToParts(turn: AgentTurn): ChatMessagePart[] {
  const parts: ChatMessagePart[] = [];
  if (turn.user) parts.push(...blockToParts(turn.user));
  for (const block of turn.process) parts.push(...blockToParts(block));
  if (turn.answer) {
    const answerParts = blockToParts(turn.answer);
    if (answerParts.length > 0) {
      const last = answerParts[answerParts.length - 1];
      if (last.kind === "text") last.isAnswer = true;
    }
    parts.push(...answerParts);
  }
  return parts;
}
