import { describe, it, expect, beforeEach } from "vitest";
import { useChatUiStore } from "../store/chatUiStore";

describe("chatUiStore", () => {
  beforeEach(() => {
    try {
      localStorage.clear();
    } catch {
      /* jsdom may lack localStorage in some setups */
    }
    useChatUiStore.setState({
      contextFocus: null,
      contextFocusSeq: 0,
      userMessageStyle: "plain",
      toolMarkdown: true,
    });
  });

  it("toggles user message style and persists", () => {
    expect(useChatUiStore.getState().userMessageStyle).toBe("plain");
    useChatUiStore.getState().toggleUserMessageStyle();
    expect(useChatUiStore.getState().userMessageStyle).toBe("bubble");
    if (typeof localStorage !== "undefined") {
      expect(localStorage.getItem("chat.userMessageStyle")).toBe("bubble");
    }
    useChatUiStore.getState().toggleUserMessageStyle();
    expect(useChatUiStore.getState().userMessageStyle).toBe("plain");
  });

  it("toggles tool markdown preference and persists", () => {
    expect(useChatUiStore.getState().toolMarkdown).toBe(true);
    useChatUiStore.getState().toggleToolMarkdown();
    expect(useChatUiStore.getState().toolMarkdown).toBe(false);
    if (typeof localStorage !== "undefined") {
      expect(localStorage.getItem("chat.toolMarkdown")).toBe("0");
    }
    useChatUiStore.getState().toggleToolMarkdown();
    expect(useChatUiStore.getState().toolMarkdown).toBe(true);
  });

  it("openContextForTool stores focus fingerprint and bumps seq", () => {
    useChatUiStore.getState().openContextForTool({
      toolName: "read_file",
      argsShort: "path=a.ts",
      outputHint: "export const x",
    });
    const s = useChatUiStore.getState();
    expect(s.contextFocus?.toolName).toBe("read_file");
    expect(s.contextFocus?.argsShort).toBe("path=a.ts");
    expect(s.contextFocusSeq).toBe(1);
  });
});
