import { describe, expect, it } from "vitest";
import {
  applyLiveTransportToParts,
  extractInProgressProcessBlocks,
  isLiveProcessActive,
  liveHasProcessPanel,
  liveProcessSummary,
  resolveLiveProcessParts,
} from "../tabs/chat/liveParts";
import type { ChatMessagePart } from "../tabs/chat/messageParts";

describe("liveParts", () => {
  const sampleLines = [
    "you> hello",
    "  … thinking",
    "  → bash_run(command=ls)",
    "  ✓ bash_run(command=ls)(12ms)",
    "assistant> partial",
  ];

  it("extractInProgressProcessBlocks takes tail after last user", () => {
    const blocks = extractInProgressProcessBlocks(sampleLines, {}, {});
    expect(blocks.length).toBeGreaterThan(0);
    expect(blocks.some((b) => b.type === "reasoning")).toBe(true);
  });

  it("applyLiveTransportToParts appends running tool", () => {
    const parts: ChatMessagePart[] = [];
    const next = applyLiveTransportToParts(parts, {
      streaming: null,
      reasoning: null,
      toolRunning: "grep",
      toolRunningDetail: "23s",
      toolPending: null,
      compressing: false,
      activityFlow: null,
      turnPhase: "tool",
    });
    expect(next).toHaveLength(1);
    expect(next[0]).toMatchObject({
      kind: "tool",
      group: { toolName: "grep", status: "running", args: null },
    });
  });

  it("applyLiveTransportToParts does not overwrite args with elapsed detail", () => {
    const parts: ChatMessagePart[] = [
      {
        id: "t1",
        kind: "tool",
        blockKey: "t1",
        group: {
          toolName: "bash_run",
          status: "running",
          ms: null,
          args: "command=ls -la",
          steps: [],
        },
      },
    ];
    const next = applyLiveTransportToParts(parts, {
      streaming: null,
      reasoning: null,
      toolRunning: "bash_run",
      toolRunningDetail: "23s",
      toolPending: null,
      compressing: false,
      activityFlow: null,
      turnPhase: "tool",
    });
    expect(next[0]).toMatchObject({
      kind: "tool",
      group: { toolName: "bash_run", status: "running", args: "command=ls -la" },
    });
  });

  it("liveProcessSummary prefers stats over generic label", () => {
    const parts: ChatMessagePart[] = [
      { id: "t1", kind: "tool", blockKey: "t1", group: { toolName: "grep", status: "ok", ms: "5", args: null, steps: [] } },
    ];
    const label = liveProcessSummary(parts, {
      streaming: null,
      reasoning: null,
      toolRunning: null,
      toolRunningDetail: null,
      toolPending: null,
      compressing: false,
      activityFlow: null,
      turnPhase: null,
    });
    expect(label).toContain("工具");
  });

  it("liveHasProcessPanel is true for live-only activity", () => {
    expect(
      liveHasProcessPanel([], {
        streaming: null,
        reasoning: null,
        toolRunning: null,
        toolRunningDetail: null,
        toolPending: null,
        compressing: false,
        activityFlow: { kind: "Skill", text: "load" },
        turnPhase: "activity",
      }),
    ).toBe(true);
  });

  it("resolveLiveProcessParts prefers ws parts over lines", () => {
    const wsParts: ChatMessagePart[] = [
      {
        id: "t1",
        kind: "tool",
        blockKey: "t1",
        group: { toolName: "grep", status: "ok", ms: "1", args: null, steps: [] },
      },
    ];
    const fromWs = resolveLiveProcessParts(wsParts, ["you> hi"], {}, {}, {
      streaming: null,
      reasoning: null,
      toolRunning: null,
      toolRunningDetail: null,
      toolPending: null,
      compressing: false,
      activityFlow: null,
      turnPhase: null,
    });
    expect(fromWs).toHaveLength(1);
    expect(fromWs[0].kind).toBe("tool");
  });

  it("isLiveProcessActive is true for running tool part", () => {
    const parts: ChatMessagePart[] = [
      {
        id: "t1",
        kind: "tool",
        blockKey: "t1",
        group: { toolName: "grep", status: "running", ms: null, args: null, steps: [] },
      },
    ];
    expect(
      isLiveProcessActive(parts, {
        reasoning: null,
        streaming: null,
        toolRunning: null,
        toolPending: null,
        compressing: false,
        activityFlow: null,
      }),
    ).toBe(true);
  });

  it("liveProcessSummary includes elapsed duration", () => {
    const withStats = liveProcessSummary(
      [
        {
          id: "t1",
          kind: "tool",
          blockKey: "t1",
          group: { toolName: "grep", status: "ok", ms: "5", args: null, steps: [] },
        },
      ],
      {
        streaming: null,
        reasoning: null,
        toolRunning: null,
        toolRunningDetail: null,
        toolPending: null,
        compressing: false,
        activityFlow: null,
        turnPhase: null,
      },
      3200,
    );
    expect(withStats).toContain("3.2s");
  });

  it("resolveLiveProcessParts merges empty WS tool steps from lines", () => {
    const lines = [
      "you> hi",
      "  → bash_run(command=ls)",
      "  ✓ bash_run(command=ls)(12ms)",
    ];
    const outputs = { "2": "a.ts\nb.ts" };
    const wsParts: ChatMessagePart[] = [
      {
        id: "tool-1-bash_run",
        kind: "tool",
        blockKey: "tool-1-bash_run",
        group: {
          toolName: "bash_run",
          status: "ok",
          ms: "12",
          args: "command=ls",
          steps: [],
        },
      },
    ];
    const parts = resolveLiveProcessParts(wsParts, lines, outputs, {}, {
      streaming: null,
      reasoning: null,
      toolRunning: null,
      toolRunningDetail: null,
      toolPending: null,
      compressing: false,
      activityFlow: null,
      turnPhase: null,
    });
    expect(parts[0].kind).toBe("tool");
    if (parts[0].kind === "tool") {
      expect(parts[0].group.steps.some((s) => s.output === "a.ts\nb.ts")).toBe(true);
    }
  });
});
