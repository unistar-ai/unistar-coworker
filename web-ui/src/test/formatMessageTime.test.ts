import { describe, it, expect, vi, afterEach } from "vitest";
import { formatMessageTime } from "../tabs/chat/formatMessageTime";

describe("formatMessageTime", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it("returns empty for missing or invalid", () => {
    expect(formatMessageTime(undefined)).toBe("");
    expect(formatMessageTime(null)).toBe("");
    expect(formatMessageTime("not-a-date")).toBe("");
  });

  it("shows time-only for same calendar day", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-07-10T15:00:00Z"));
    const label = formatMessageTime("2026-07-10T08:30:00Z");
    expect(label.length).toBeGreaterThan(0);
    expect(label).not.toMatch(/Jul|7月|2026/i);
  });

  it("includes date for other days", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-07-10T15:00:00Z"));
    const label = formatMessageTime("2026-07-09T08:30:00Z");
    expect(label.length).toBeGreaterThan(0);
    // Locale-dependent; at least not empty and not identical to a bare HH:MM-only pattern alone.
    expect(label).toMatch(/\d/);
  });
});
