import { describe, expect, it } from "vitest";
import {
  estimateItemSize,
  isNewUserTurn,
  itemKey,
  TURN_ESTIMATE_SIZE_PX,
  BLOCK_ESTIMATE_SIZE_PX,
} from "../tabs/chat/chatScroll";
import type { ChatHistoryItem } from "../tabs/chat/parser";

describe("chatScroll", () => {
  it("itemKey distinguishes turn vs block", () => {
    const turn: ChatHistoryItem = {
      type: "turn",
      turn: { key: "turn-1", process: [] },
    };
    const block: ChatHistoryItem = {
      type: "block",
      block: { type: "meta", key: "b-1" },
    };
    expect(itemKey(turn)).toBe("turn-1");
    expect(itemKey(block)).toBe("b-1");
  });

  it("estimateItemSize uses turn vs block heights", () => {
    const turn: ChatHistoryItem = {
      type: "turn",
      turn: { key: "t", process: [] },
    };
    const block: ChatHistoryItem = {
      type: "block",
      block: { type: "meta", key: "b" },
    };
    expect(estimateItemSize(turn)).toBe(TURN_ESTIMATE_SIZE_PX);
    expect(estimateItemSize(block)).toBe(BLOCK_ESTIMATE_SIZE_PX);
  });

  it("isNewUserTurn requires turn with user and new key flag", () => {
    const withUser: ChatHistoryItem = {
      type: "turn",
      turn: {
        key: "t",
        user: { type: "message", key: "u", message: { role: "you", badge: "", body: "hi", lineIndex: 0, md: false } },
        process: [],
      },
    };
    const agentOnly: ChatHistoryItem = {
      type: "turn",
      turn: { key: "t", process: [] },
    };
    expect(isNewUserTurn(withUser, true)).toBe(true);
    expect(isNewUserTurn(withUser, false)).toBe(false);
    expect(isNewUserTurn(agentOnly, true)).toBe(false);
  });
});
