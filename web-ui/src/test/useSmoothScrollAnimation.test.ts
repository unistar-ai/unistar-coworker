import { describe, expect, it, vi } from "vitest";
import { renderHook, act } from "@testing-library/react";
import { useRef } from "react";
import { useSmoothScrollAnimation } from "../tabs/chat/useSmoothScrollAnimation";

describe("useSmoothScrollAnimation", () => {
  it("followTo moves scrollTop toward target", () => {
    const el = document.createElement("div");
    Object.defineProperty(el, "scrollTop", { writable: true, value: 0 });
    Object.defineProperty(el, "scrollHeight", { value: 1000 });
    Object.defineProperty(el, "clientHeight", { value: 400 });

    const frames: FrameRequestCallback[] = [];
    const raf = vi.fn((cb: FrameRequestCallback) => {
      frames.push(cb);
      return frames.length;
    });
    const caf = vi.fn();

    const { result } = renderHook(() => {
      const ref = useRef<HTMLElement | null>(el);
      return useSmoothScrollAnimation(ref, { raf, caf });
    });

    act(() => {
      result.current.followTo(() => 600);
    });

    expect(frames.length).toBeGreaterThan(0);
    act(() => {
      for (let i = 0; i < 30 && frames.length; i++) {
        const cb = frames.shift();
        cb?.(i * 16);
      }
    });
    expect(el.scrollTop).toBeGreaterThan(0);
  });

  it("cancel stops animation", () => {
    const el = document.createElement("div");
    Object.defineProperty(el, "scrollTop", { writable: true, value: 0 });

    const raf = vi.fn(() => 1);
    const caf = vi.fn();

    const { result } = renderHook(() => {
      const ref = useRef<HTMLElement | null>(el);
      return useSmoothScrollAnimation(ref, { raf, caf });
    });

    act(() => {
      result.current.followTo(() => 500);
      result.current.cancel();
    });

    expect(caf).toHaveBeenCalled();
    expect(result.current.isAnimating()).toBe(false);
  });
});
