import { describe, expect, it } from "vitest";
import {
  isProcessPart,
  partsProcessStats,
  resolveHistoryAgentParts,
  splitTurnParts,
  turnAgentParts,
  turnBlocksToParts,
} from "../tabs/chat/messageParts";
import type { AgentTurn } from "../tabs/chat/parser";
import type { ChatMessagePart } from "../tabs/chat/messageParts";

function sampleTurn(): AgentTurn {
  return {
    key: "turn-0",
    user: {
      type: "message",
      key: "u1",
      message: { role: "you", badge: "", body: "question", lineIndex: 0, md: false },
    },
    process: [
      {
        type: "reasoning",
        key: "r1",
        reasoningText: "from-parser",
      },
    ],
    answer: {
      type: "message",
      key: "a1",
      message: { role: "assistant", badge: "", body: "done", lineIndex: 3, md: true },
    },
  };
}

describe("resolveHistoryAgentParts", () => {
  it("prefers backend history parts for process", () => {
    const turn = sampleTurn();
    const ws: Record<string, ChatMessagePart[]> = {
      "0": [
        { id: "reasoning-1", kind: "reasoning", text: "from-backend" },
        {
          id: "tool-2-grep",
          kind: "tool",
          blockKey: "tool-2-grep",
          group: {
            toolName: "grep",
            status: "ok",
            ms: "3",
            args: "pattern=x",
            steps: [
              { kind: "start", text: "grep", index: 2, name: "grep" },
              { kind: "done", text: "grep", index: 3, name: "grep", ok: true, output: "hit" },
            ],
          },
        },
      ],
    };
    const parts = resolveHistoryAgentParts(turn, ws);
    expect(parts[0]).toMatchObject({ kind: "reasoning", text: "from-backend" });
    expect(parts[1]).toMatchObject({ kind: "tool" });
    expect(parts[parts.length - 1]).toMatchObject({ kind: "text", isAnswer: true, text: "done" });
  });

  it("falls back to parser when no history parts", () => {
    const parts = resolveHistoryAgentParts(sampleTurn(), {});
    expect(parts[0]).toMatchObject({ kind: "reasoning", text: "from-parser" });
  });
});


describe("turnBlocksToParts", () => {
  it("orders user, process, and answer parts", () => {
    const turn: AgentTurn = {
      key: "turn-0",
      user: {
        type: "message",
        key: "u1",
        message: { role: "you", badge: "", body: "question", lineIndex: 0, md: false },
      },
      process: [
        {
          type: "reasoning",
          key: "r1",
          reasoningText: "thinking",
        },
        {
          type: "tool-group",
          key: "tg1",
          group: {
            toolName: "bash_run",
            status: "ok",
            ms: "10",
            args: "ls",
            steps: [],
          },
        },
      ],
      answer: {
        type: "message",
        key: "a1",
        message: { role: "assistant", badge: "", body: "done", lineIndex: 3, md: true },
      },
    };

    const parts = turnBlocksToParts(turn);
    expect(parts.map((p) => p.kind)).toEqual([
      "text",
      "reasoning",
      "tool",
      "text",
    ]);
    expect(parts[0]).toMatchObject({ kind: "text", role: "user", text: "question" });
    expect(parts[1]).toMatchObject({ kind: "reasoning", text: "thinking" });
    expect(parts[2]).toMatchObject({ kind: "tool", blockKey: "tg1" });
    const answer = parts[3];
    expect(answer).toMatchObject({ kind: "text", role: "assistant", text: "done", isAnswer: true });
  });

  it("marks interim assistant messages in process without isAnswer", () => {
    const turn: AgentTurn = {
      key: "turn-1",
      process: [
        {
          type: "message",
          key: "m1",
          message: { role: "assistant", badge: "", body: "interim", lineIndex: 1, md: false },
        },
      ],
    };
    const parts = turnBlocksToParts(turn);
    expect(parts).toHaveLength(1);
    expect(parts[0]).toMatchObject({ kind: "text", text: "interim" });
    expect((parts[0] as { isAnswer?: boolean }).isAnswer).toBeUndefined();
  });
});

describe("splitTurnParts", () => {
  const turn: AgentTurn = {
    key: "turn-2",
    process: [
      { type: "reasoning", key: "r1", reasoningText: "think" },
      {
        type: "tool-group",
        key: "t1",
        group: { toolName: "grep", status: "ok", ms: "5", args: null, steps: [] },
      },
    ],
    answer: {
      type: "message",
      key: "a1",
      message: { role: "assistant", badge: "", body: "answer", lineIndex: 2, md: false },
    },
  };

  it("splits process from marked answer part", () => {
    const parts = turnAgentParts(turn);
    const { processParts, answerParts } = splitTurnParts(parts);
    expect(processParts).toHaveLength(2);
    expect(answerParts).toHaveLength(1);
    expect(answerParts[0]).toMatchObject({ kind: "text", isAnswer: true });
  });

  it("counts tools and thoughts in process parts", () => {
    const parts = turnAgentParts(turn);
    const { processParts } = splitTurnParts(parts);
    expect(partsProcessStats(processParts)).toEqual({ tools: 1, thoughts: 1 });
  });

  it("treats interim assistant text as process", () => {
    const parts = turnBlocksToParts({
      key: "t",
      process: [
        {
          type: "message",
          key: "m1",
          message: { role: "assistant", badge: "", body: "interim", lineIndex: 0, md: false },
        },
      ],
    });
    expect(isProcessPart(parts[0])).toBe(true);
    const split = splitTurnParts(parts);
    expect(split.processParts).toHaveLength(1);
    expect(split.answerParts).toHaveLength(0);
  });
});
