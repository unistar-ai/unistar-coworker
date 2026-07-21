import { describe, expect, it } from "vitest";
import { resolveLiveHeaderCandidate } from "../tabs/chat/liveHeader";
import type { ChatMessagePart } from "../tabs/chat/messageParts";
import type { LiveTransportState } from "../tabs/chat/liveParts";

const idleLive: LiveTransportState = {
  streaming: null,
  reasoning: null,
  toolRunning: null,
  toolRunningDetail: null,
  toolPending: null,
  compressing: false,
  activityFlow: null,
  turnPhase: null,
};

describe("resolveLiveHeaderCandidate", () => {
  it("shows tool label while a tool is running", () => {
    const c = resolveLiveHeaderCandidate([], { ...idleLive, toolRunning: "bash_run" }, 1200);
    expect(c.text).toBe("Bash");
    expect(c.shimmer).toBe(true);
  });

  it("shows thinking while reasoning streams", () => {
    const c = resolveLiveHeaderCandidate([], { ...idleLive, reasoning: "…" }, null);
    expect(c.text).toBe("深度思考中");
    expect(c.shimmer).toBe(true);
  });

  it("prefers settled summary when preferSummary", () => {
    const parts: ChatMessagePart[] = [
      {
        id: "t1",
        kind: "tool",
        blockKey: "t1",
        group: { toolName: "grep", status: "ok", ms: "5", args: null, steps: [] },
      },
    ];
    const c = resolveLiveHeaderCandidate(
      parts,
      { ...idleLive, toolRunning: "grep" },
      3200,
      [],
      true,
    );
    expect(c.shimmer).toBe(false);
    expect(c.text).toContain("工具");
    expect(c.text).toContain("3.2s");
  });
});
