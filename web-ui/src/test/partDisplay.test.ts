import { describe, expect, it } from "vitest";
import { partsToDisplaySteps } from "../tabs/chat/partDisplay";
import type { ChatMessagePart } from "../tabs/chat/messageParts";

describe("partsToDisplaySteps", () => {
  const mcpPrefixes: { id: string; prefix: string }[] = [];

  it("maps reasoning and tool parts to display rows", () => {
    const parts: ChatMessagePart[] = [
      { id: "r1", kind: "reasoning", text: "line one\nline two" },
      {
        id: "t1",
        kind: "tool",
        blockKey: "t1",
        group: { toolName: "bash_run", status: "ok", ms: "12", args: "command=ls", steps: [] },
      },
    ];
    const steps = partsToDisplaySteps(parts, mcpPrefixes);
    expect(steps).toHaveLength(2);
    expect(steps[0].kind).toBe("thought");
    expect(steps[0].title).toBe("深度思考");
    expect(steps[1].kind).toBe("tool");
    expect(steps[1].title).toBe("执行命令");
    expect(steps[1].toolName).toBe("bash_run");
  });

  it("uses thought title and one-line preview", () => {
    const parts: ChatMessagePart[] = [
      { id: "r1", kind: "reasoning", text: "First line\nSecond line of thought" },
    ];
    const steps = partsToDisplaySteps(parts, []);
    expect(steps[0].title).toBe("深度思考");
    expect(steps[0].subtitle).toContain("First line");
    expect(steps[0].subtitle).not.toContain("\n");
  });

  it("shows ms for completed tools instead of 已完成", () => {
    const parts: ChatMessagePart[] = [
      {
        id: "t1",
        kind: "tool",
        blockKey: "t1",
        group: {
          toolName: "bash_run",
          status: "ok",
          ms: "42",
          args: "command=ls",
          steps: [],
        },
      },
    ];
    const steps = partsToDisplaySteps(parts, []);
    expect(steps[0].status).toBe("42ms");
    expect(steps[0].statusKind).toBe("ok");
  });

  it("maps running statusKind correctly", () => {
    const parts: ChatMessagePart[] = [
      {
        id: "t1",
        kind: "tool",
        blockKey: "t1",
        group: {
          toolName: "grep",
          status: "running",
          ms: null,
          args: "pattern=foo, path=src",
          steps: [],
        },
      },
    ];
    const steps = partsToDisplaySteps(parts, []);
    expect(steps[0].statusKind).toBe("running");
    expect(steps[0].status).toBe("执行中");
    expect(steps[0].title).toBe("搜索代码");
    expect(steps[0].subtitle).toContain("foo");
  });

  it("shows ask_user answer from tool output", () => {
    const parts: ChatMessagePart[] = [
      {
        id: "t1",
        kind: "tool",
        blockKey: "t1",
        group: {
          toolName: "ask_user",
          status: "ok",
          ms: "5",
          args: "question=Which repo?",
          steps: [
            {
              kind: "done",
              name: "ask_user",
              ok: true,
              output: "User answered:\nunistar/unistar-coworker",
            },
          ],
        },
      },
    ];
    const steps = partsToDisplaySteps(parts, []);
    expect(steps[0].title).toBe("向用户提问");
    expect(steps[0].subtitle).toBe("unistar/unistar-coworker");
  });

  it("maps interim assistant text to comment rows, not 执行中", () => {
    const parts: ChatMessagePart[] = [
      {
        id: "n1",
        kind: "text",
        role: "assistant",
        text: "让我先检查 GitHub CLI 的状态。",
      },
    ];
    const steps = partsToDisplaySteps(parts, []);
    expect(steps[0].kind).toBe("comment");
    expect(steps[0].title).toBe("说明");
    expect(steps[0].subtitle).toContain("GitHub CLI");
  });

  it("dedupes consecutive identical tool rows", () => {
    const group = {
      toolName: "ask_user",
      status: "ok" as const,
      ms: "5",
      args: "question=Which repo?",
      steps: [
        {
          kind: "done" as const,
          name: "ask_user",
          ok: true,
          output: "User answered:\nunistar/unistar-coworker",
        },
      ],
    };
    const parts: ChatMessagePart[] = [
      { id: "t1", kind: "tool", blockKey: "t1", group },
      { id: "t2", kind: "tool", blockKey: "t2", group },
    ];
    const steps = partsToDisplaySteps(parts, []);
    expect(steps).toHaveLength(1);
  });
});
