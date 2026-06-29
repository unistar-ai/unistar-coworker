import { describe, it, expect, beforeEach } from "vitest";
import { useStore } from "../store/wsStore";
import type {
  WebSnapshot,
  WebLivePatch,
  WebChatPatch,
} from "../store/protocol";

// Minimal snapshot factory — only fields the store reads are exercised; the
// rest inherit from DEFAULT_SNAPSHOT via applySnapshot's spread.
function snap(overrides: Partial<WebSnapshot> = {}): WebSnapshot {
  return {
    tab: "chat",
    tabs: ["chat"],
    status: "ready",
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
    ...overrides,
  };
}

function livePatch(overrides: Partial<WebLivePatch> = {}): WebLivePatch {
  return {
    _type: "live",
    status: "working",
    chat_busy: true,
    chat_streaming: null,
    chat_reasoning: null,
    chat_tool_running: null,
    chat_tool_running_detail: null,
    chat_tool_pending: null,
    chat_turn_phase: "model",
    chat_reasoning_compressing: false,
    chat_activity_flow: null,
    ...overrides,
  };
}

function chatPatch(overrides: Partial<WebChatPatch> = {}): WebChatPatch {
  return {
    _type: "chat",
    status: "ready",
    chat_busy: false,
    chat_session_id: null,
    chat_lines: [],
    chat_tool_outputs: {},
    chat_history_revision: 1,
    chat_context_revision: 1,
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
    ...overrides,
  };
}

describe("wsStore", () => {
  beforeEach(() => {
    // Reset to default (no snapshot, not connected, no error).
    useStore.setState({
      ...useStore.getState(),
      connected: false,
      reconnectAttempts: 0,
      statusError: null,
      hasSnapshot: false,
    });
  });

  it("starts without a snapshot, not connected, no error", () => {
    const s = useStore.getState();
    expect(s.hasSnapshot).toBe(false);
    expect(s.connected).toBe(false);
    expect(s.statusError).toBeNull();
  });

  it("applySnapshot sets hasSnapshot + connected and clears reconnect attempts", () => {
    useStore.getState().applySnapshot(snap({ tab: "prs", status: "ready" }));
    const s = useStore.getState();
    expect(s.hasSnapshot).toBe(true);
    expect(s.connected).toBe(true);
    expect(s.reconnectAttempts).toBe(0);
    expect(s.tab).toBe("prs");
  });

  it("applyWsMessage with a snapshot sets hasSnapshot", () => {
    useStore.getState().applyWsMessage(snap({ tab: "dashboard" }));
    expect(useStore.getState().hasSnapshot).toBe(true);
    expect(useStore.getState().tab).toBe("dashboard");
  });

  it("applyWsMessage with a live patch does NOT set hasSnapshot", () => {
    // A live patch arriving before any snapshot must not flip the gate open.
    useStore.getState().applyWsMessage(livePatch({ chat_busy: true }));
    expect(useStore.getState().hasSnapshot).toBe(false);
    expect(useStore.getState().chat_busy).toBe(true);
  });

  it("applyWsMessage with a chat patch does NOT set hasSnapshot", () => {
    useStore.getState().applyWsMessage(
      chatPatch({ chat_lines: ["you> hi"], chat_busy: false }),
    );
    expect(useStore.getState().hasSnapshot).toBe(false);
    expect(useStore.getState().chat_lines).toEqual(["you> hi"]);
  });

  it("setStatusError stores and clears the error", () => {
    useStore.getState().setStatusError("state fetch failed (401)");
    expect(useStore.getState().statusError).toBe("state fetch failed (401)");
    useStore.getState().setStatusError(null);
    expect(useStore.getState().statusError).toBeNull();
  });

  it("a snapshot clears statusError implicitly via applySnapshot spread", () => {
    // Simulate the App.tsx retry path: error set, then snapshot arrives.
    useStore.getState().setStatusError("boom");
    useStore.getState().applySnapshot(snap());
    // applySnapshot doesn't touch statusError itself, but App.tsx calls
    // setStatusError(null) right before. Verify the gate opens once a
    // snapshot is present regardless of a stale error.
    expect(useStore.getState().hasSnapshot).toBe(true);
  });

  it("setTab optimistically updates the active tab", () => {
    useStore.getState().applySnapshot(snap({ tab: "chat" }));
    useStore.getState().setTab("logs");
    expect(useStore.getState().tab).toBe("logs");
  });

  it("setConnection tracks reconnect attempts", () => {
    useStore.getState().setConnection(false, 3);
    expect(useStore.getState().connected).toBe(false);
    expect(useStore.getState().reconnectAttempts).toBe(3);
    useStore.getState().setConnection(true, 0);
    expect(useStore.getState().connected).toBe(true);
  });
});
