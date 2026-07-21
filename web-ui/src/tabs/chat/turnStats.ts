import type { ChatMessage } from "./parser";
import { formatDurationMs, formatTokens, turnProcessDurationMs } from "./parser";
import type { AgentTurn, ChatBlock } from "./parser";

/** Rough token estimate for display (not billing-grade). */
export function estimateTextTokens(text: string): number {
  const t = text.trim();
  if (!t) return 0;
  return Math.max(1, Math.ceil(t.length / 4));
}

export function estimateTurnTokens(turn: AgentTurn): number {
  let total = 0;
  if (turn.user?.message?.body) {
    total += estimateTextTokens(turn.user.message.body);
  }
  for (const b of turn.process) {
    if (b.type === "message" && b.message?.body) {
      total += estimateTextTokens(b.message.body);
    }
    if (b.type === "reasoning" && b.reasoningText) {
      total += estimateTextTokens(b.reasoningText);
    }
    if (b.type === "tool-group" && b.group) {
      for (const s of b.group.steps) {
        if (s.output) total += estimateTextTokens(s.output);
      }
    }
  }
  if (turn.answer?.message?.body) {
    total += estimateTextTokens(turn.answer.message.body);
  }
  return total;
}

export function formatAssistantTurnMeta(parts: {
  durationMs?: number | null;
  tokenEstimate?: number;
}): string | null {
  const bits: string[] = [];
  if (parts.durationMs != null && parts.durationMs > 0) {
    bits.push(formatDurationMs(parts.durationMs));
  }
  if (parts.tokenEstimate != null && parts.tokenEstimate > 0) {
    bits.push(`~${formatTokens(parts.tokenEstimate)} tokens`);
  }
  return bits.length ? bits.join(" · ") : null;
}

export function turnMetaFromAgentTurn(turn: AgentTurn): string | null {
  const durationMs = turnProcessDurationMs(turn.process);
  const tokenEstimate = estimateTurnTokens(turn);
  return formatAssistantTurnMeta({ durationMs, tokenEstimate });
}

export function turnMetaFromMessage(
  process: ChatBlock[],
  answer?: ChatMessage,
): string | null {
  const durationMs = turnProcessDurationMs(process);
  let tokens = 0;
  if (answer?.body) tokens += estimateTextTokens(answer.body);
  return formatAssistantTurnMeta({ durationMs, tokenEstimate: tokens || undefined });
}
