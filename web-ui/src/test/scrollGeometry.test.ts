import { describe, expect, it } from "vitest";
import {
  getDistanceToBottom,
  getRealBottom,
  isMoreThanOneViewportFromBottom,
} from "../tabs/chat/scrollGeometry";

describe("scrollGeometry", () => {
  const el = {
    clientHeight: 400,
    scrollHeight: 1000,
    scrollTop: 500,
  };

  it("getRealBottom returns scrollable range", () => {
    expect(getRealBottom(el)).toBe(600);
  });

  it("getDistanceToBottom measures gap to live edge", () => {
    expect(getDistanceToBottom(el)).toBe(100);
  });

  it("isMoreThanOneViewportFromBottom when far behind", () => {
    expect(isMoreThanOneViewportFromBottom({ ...el, scrollTop: 0 })).toBe(true);
    expect(isMoreThanOneViewportFromBottom({ ...el, scrollTop: 550 })).toBe(false);
  });
});
