import { describe, expect, it } from "vitest";
import { formatAssistantTurnMeta, estimateTextTokens } from "../tabs/chat/turnStats";

describe("turnStats", () => {
  it("estimateTextTokens rounds up by char/4", () => {
    expect(estimateTextTokens("abcd")).toBe(1);
    expect(estimateTextTokens("a".repeat(9))).toBe(3);
  });

  it("formatAssistantTurnMeta joins duration and tokens", () => {
    expect(formatAssistantTurnMeta({ durationMs: 3200, tokenEstimate: 1200 })).toBe(
      "3.2s · ~1.20k tokens",
    );
  });
});
