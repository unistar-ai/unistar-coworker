import { describe, expect, it } from "vitest";
import { shouldShowStreamingAnswer } from "../tabs/chat/useTrailingAnswerRelease";

describe("shouldShowStreamingAnswer", () => {
  it("shows while process is still active when streaming text exists", () => {
    expect(shouldShowStreamingAnswer(true, "hello", false, true)).toBe(true);
  });

  it("shows immediately when no prior process", () => {
    expect(shouldShowStreamingAnswer(false, "hello", false, false)).toBe(true);
  });

  it("shows without waiting for release after process", () => {
    expect(shouldShowStreamingAnswer(false, "hello", false, true)).toBe(true);
    expect(shouldShowStreamingAnswer(false, "hello", true, true)).toBe(true);
  });

  it("hides when streaming is empty", () => {
    expect(shouldShowStreamingAnswer(false, null, true, false)).toBe(false);
    expect(shouldShowStreamingAnswer(false, "", true, false)).toBe(false);
  });
});
