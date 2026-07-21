import { describe, expect, it } from "vitest";
import { shouldShowStreamingAnswer } from "../tabs/chat/useTrailingAnswerRelease";

describe("shouldShowStreamingAnswer", () => {
  it("hides while process is active", () => {
    expect(shouldShowStreamingAnswer(true, "hello", false, true)).toBe(false);
  });

  it("shows immediately when no prior process", () => {
    expect(shouldShowStreamingAnswer(false, "hello", false, false)).toBe(true);
  });

  it("waits for release after process", () => {
    expect(shouldShowStreamingAnswer(false, "hello", false, true)).toBe(false);
    expect(shouldShowStreamingAnswer(false, "hello", true, true)).toBe(true);
  });

  it("hides when streaming is empty", () => {
    expect(shouldShowStreamingAnswer(false, null, true, false)).toBe(false);
  });
});
