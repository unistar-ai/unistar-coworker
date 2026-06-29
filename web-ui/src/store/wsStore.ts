import { create } from "zustand";
import type {
  WebSnapshot,
  WebLivePatch,
  WebChatPatch,
  WsMessage,
} from "./protocol";
import { isLivePatch, isChatPatch, isSnapshot } from "./protocol";

// Default snapshot used before the first WS message arrives. Mirrors the
// shape of a fresh AppState on the Rust side.
const DEFAULT_SNAPSHOT: WebSnapshot = {
  tab: "chat",
  tabs: [],
  status: "connecting…",
  engine_busy: false,
  engine_workflow_id: null,
  chat_enabled: true,
  chat_busy: false,
  chat_session_id: null,
  chat_lines: [],
  chat_tool_outputs: {},
  chat_history_revision: 0,
  chat_context_revision: 0,
  chat_streaming: null,
  chat_reasoning: null,
  chat_tool_running: null,
  chat_tool_running_detail: null,
  chat_tool_pending: null,
  chat_turn_phase: null,
  chat_reasoning_compressing: false,
  chat_activity_flow: null,
  chat_context_visible: false,
  chat_context: {
    turn: 0,
    message_tokens: 0,
    tools_tokens: 0,
    tools_body: "",
    tool_names: [],
    skills_tokens: 0,
    skill_blocks: [],
    input_budget: 60000,
    context_limit: 64000,
    message_count: 0,
    messages: [],
    runtime_context_revision: null,
    context_trimmed_turns: 0,
    context_summary_note: null,
  },
  chat_pending_approval: null,
  approval_dialog: null,
  digest_history: [],
  digest_bodies: {},
  selected_digest_date: null,
  prs: [],
  pr_filter: "all",
  pr_sort: "default",
  selected_pr_index: 0,
  pr_overview: null,
  pr_overview_loading: false,
  approvals: [],
  log_filter: "all",
  logs: [],
  config_path: "",
  repos: [],
  llm_model: "",
  github_ok: false,
  llm_ok: false,
  github_latency_ms: null,
  llm_latency_ms: null,
  mcp_servers: [],
  attach_mode: false,
  auto_approve_mutations: false,
  ui_theme: "dark",
};

interface UiState {
  connected: boolean;
  reconnectAttempts: number;
  statusError: string | null;
  /** True once a snapshot (initial /api/state or first WS snapshot) has been
   * applied. Used by the StateGate to distinguish "still loading" from
   * "loaded but disconnected". */
  hasSnapshot: boolean;
}

interface Store extends WebSnapshot, UiState {
  applySnapshot: (s: WebSnapshot) => void;
  applyLivePatch: (p: WebLivePatch) => void;
  applyChatPatch: (p: WebChatPatch) => void;
  applyWsMessage: (m: WsMessage) => void;
  setConnection: (connected: boolean, attempts: number) => void;
  setStatusError: (msg: string | null) => void;
  setTab: (tab: string) => void;
  /** Chat input draft — persisted across tab switches in the store. */
  chatDraft: string;
  setChatDraft: (draft: string) => void;
}

export const useStore = create<Store>((set) => ({
  ...DEFAULT_SNAPSHOT,
  connected: false,
  reconnectAttempts: 0,
  statusError: null,
  hasSnapshot: false,
  chatDraft: "",

  applySnapshot: (s) =>
    set(() => ({ ...s, connected: true, reconnectAttempts: 0, hasSnapshot: true })),

  applyLivePatch: (p) =>
    set(() => ({
      status: p.status,
      chat_busy: p.chat_busy,
      chat_streaming: p.chat_streaming,
      chat_reasoning: p.chat_reasoning,
      chat_tool_running: p.chat_tool_running,
      chat_tool_running_detail: p.chat_tool_running_detail,
      chat_tool_pending: p.chat_tool_pending,
      chat_turn_phase: p.chat_turn_phase,
      chat_reasoning_compressing: p.chat_reasoning_compressing,
      chat_activity_flow: p.chat_activity_flow,
    })),

  applyChatPatch: (p) =>
    set((state) => ({
      status: p.status,
      chat_busy: p.chat_busy,
      chat_session_id: p.chat_session_id ?? state.chat_session_id,
      chat_lines: p.chat_lines,
      chat_tool_outputs: p.chat_tool_outputs,
      chat_history_revision: p.chat_history_revision,
      chat_context_revision: p.chat_context_revision,
      chat_streaming: p.chat_streaming,
      chat_reasoning: p.chat_reasoning,
      chat_tool_running: p.chat_tool_running,
      chat_tool_running_detail: p.chat_tool_running_detail,
      chat_tool_pending: p.chat_tool_pending,
      chat_turn_phase: p.chat_turn_phase,
      chat_reasoning_compressing: p.chat_reasoning_compressing,
      chat_activity_flow: p.chat_activity_flow,
      chat_context_visible: p.chat_context_visible,
      chat_context: p.chat_context,
      chat_pending_approval: p.chat_pending_approval,
      approval_dialog: p.approval_dialog,
    })),

  applyWsMessage: (m) => {
    if (isSnapshot(m)) {
      set(() => ({ ...m, connected: true, reconnectAttempts: 0, hasSnapshot: true }));
    } else if (isLivePatch(m)) {
      set(() => ({
        status: m.status,
        chat_busy: m.chat_busy,
        chat_streaming: m.chat_streaming,
        chat_reasoning: m.chat_reasoning,
        chat_tool_running: m.chat_tool_running,
        chat_tool_running_detail: m.chat_tool_running_detail,
        chat_tool_pending: m.chat_tool_pending,
        chat_turn_phase: m.chat_turn_phase,
        chat_reasoning_compressing: m.chat_reasoning_compressing,
        chat_activity_flow: m.chat_activity_flow,
      }));
    } else if (isChatPatch(m)) {
      set((state) => ({
        status: m.status,
        chat_busy: m.chat_busy,
        chat_session_id: m.chat_session_id ?? state.chat_session_id,
        chat_lines: m.chat_lines,
        chat_tool_outputs: m.chat_tool_outputs,
        chat_history_revision: m.chat_history_revision,
        chat_context_revision: m.chat_context_revision,
        chat_streaming: m.chat_streaming,
        chat_reasoning: m.chat_reasoning,
        chat_tool_running: m.chat_tool_running,
        chat_tool_running_detail: m.chat_tool_running_detail,
        chat_tool_pending: m.chat_tool_pending,
        chat_turn_phase: m.chat_turn_phase,
        chat_reasoning_compressing: m.chat_reasoning_compressing,
        chat_activity_flow: m.chat_activity_flow,
        chat_context_visible: m.chat_context_visible,
        chat_context: m.chat_context,
        chat_pending_approval: m.chat_pending_approval,
        approval_dialog: m.approval_dialog,
      }));
    }
  },

  setConnection: (connected, attempts) =>
    set({ connected, reconnectAttempts: attempts }),

  setStatusError: (msg) => set({ statusError: msg }),

  // Optimistic tab switch — immediately update local state so the Radix Tabs
  // component reflects the change before the WS snapshot arrives.
  setTab: (tab: string) => set({ tab }),
  setChatDraft: (draft: string) => set({ chatDraft: draft }),
}));
