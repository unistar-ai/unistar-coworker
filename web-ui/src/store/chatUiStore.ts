import { create } from "zustand";
import { apiPost } from "../lib/api";
import type { ContextToolFocus } from "../tabs/chat/contextFocus";
import { toolNamesMatch } from "../tabs/chat/contextFocus";

export type { ContextToolFocus };
export { toolNamesMatch };

export type UserMessageStyle = "plain" | "bubble";

const USER_STYLE_KEY = "chat.userMessageStyle";
const TOOL_MD_KEY = "chat.toolMarkdown";

function loadUserMessageStyle(): UserMessageStyle {
  try {
    const v = localStorage.getItem(USER_STYLE_KEY);
    if (v === "bubble" || v === "plain") return v;
  } catch {
    /* localStorage unavailable */
  }
  return "plain";
}

function loadToolMarkdown(): boolean {
  try {
    const v = localStorage.getItem(TOOL_MD_KEY);
    if (v === "0" || v === "false") return false;
    if (v === "1" || v === "true") return true;
  } catch {
    /* localStorage unavailable */
  }
  return true;
}

/** Client-only UI state (not synced over WebSocket). */
export const useChatUiStore = create<{
  contextFocus: ContextToolFocus | null;
  /** Bumped on each open so re-clicking the same tool re-runs scroll. */
  contextFocusSeq: number;
  userMessageStyle: UserMessageStyle;
  /** When true, tool results that qualify as markdown are rendered as Markdown. */
  toolMarkdown: boolean;
  openContextForTool: (focus: ContextToolFocus) => void;
  clearContextFocus: () => void;
  setUserMessageStyle: (style: UserMessageStyle) => void;
  toggleUserMessageStyle: () => void;
  setToolMarkdown: (enabled: boolean) => void;
  toggleToolMarkdown: () => void;
}>((set, get) => ({
  contextFocus: null,
  contextFocusSeq: 0,
  userMessageStyle: loadUserMessageStyle(),
  toolMarkdown: loadToolMarkdown(),
  openContextForTool: (focus) => {
    void apiPost("/api/chat/context", { visible: true });
    set((s) => ({
      contextFocus: focus,
      contextFocusSeq: s.contextFocusSeq + 1,
    }));
  },
  clearContextFocus: () => set({ contextFocus: null }),
  setUserMessageStyle: (style) => {
    try {
      localStorage.setItem(USER_STYLE_KEY, style);
    } catch {
      /* localStorage unavailable */
    }
    set({ userMessageStyle: style });
  },
  toggleUserMessageStyle: () => {
    const next = get().userMessageStyle === "plain" ? "bubble" : "plain";
    get().setUserMessageStyle(next);
  },
  setToolMarkdown: (enabled) => {
    try {
      localStorage.setItem(TOOL_MD_KEY, enabled ? "1" : "0");
    } catch {
      /* localStorage unavailable */
    }
    set({ toolMarkdown: enabled });
  },
  toggleToolMarkdown: () => {
    get().setToolMarkdown(!get().toolMarkdown);
  },
}));
