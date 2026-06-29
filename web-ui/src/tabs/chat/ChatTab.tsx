import { useEffect, useMemo, useRef, useState } from "react";
import { useStore } from "../../store/wsStore";
import { apiPost } from "../../lib/api";
import ChatHistory from "./ChatHistory";
import LiveZone, { useLiveZoneActive } from "./LiveZone";
import LiveDivider from "./LiveDivider";
import ContextPanel from "./ContextPanel";
import SessionPicker from "./SessionPicker";
import { PanelRightOpen } from "lucide-react";
import {
  buildChatBlocks,
  messageStatsFromBlocks,
  formatMessageCount,
} from "./parser";

/** True when the viewport is ≤900px (matches the CSS responsive breakpoint). */
function useIsMobile(): boolean {
  const [isMobile, setIsMobile] = useState(
    () => typeof window !== "undefined" && window.matchMedia("(max-width: 900px)").matches,
  );
  useEffect(() => {
    if (typeof window === "undefined") return;
    const mq = window.matchMedia("(max-width: 900px)");
    const onChange = (e: MediaQueryListEvent) => setIsMobile(e.matches);
    mq.addEventListener("change", onChange);
    return () => mq.removeEventListener("change", onChange);
  }, []);
  return isMobile;
}

export default function ChatTab() {
  const chatEnabled = useStore((s) => s.chat_enabled);
  const contextVisible = useStore((s) => s.chat_context_visible);
  const chatBusy = useStore((s) => s.chat_busy);
  const chatLines = useStore((s) => s.chat_lines);
  const outputs = useStore((s) => s.chat_tool_outputs);
  const autoApprove = useStore((s) => s.auto_approve_mutations);
  const chatTurnPhase = useStore((s) => s.chat_turn_phase);

  const scrollRef = useRef<HTMLDivElement>(null);
  const [stickBottom, setStickBottom] = useState(true);
  const liveActive = useLiveZoneActive();
  const isMobile = useIsMobile();
  // Local drawer state for the mobile context panel. The server-side
  // chat_context_visible flag drives the desktop column; on mobile we layer a
  // local "drawer open" toggle on top so the panel can be shown/hidden as an
  // overlay without round-tripping through the backend for every toggle.
  const [mobileCtxOpen, setMobileCtxOpen] = useState(false);

  const blocks = useMemo(
    () => buildChatBlocks(chatLines, outputs),
    [chatLines, outputs],
  );
  const stats = useMemo(() => messageStatsFromBlocks(blocks), [blocks]);
  const countLabel = formatMessageCount(stats);

  if (!chatEnabled) {
    return <div className="p-4 text-text-muted">Chat is disabled in config.</div>;
  }

  const phaseLabel = PHASE_LABELS[chatTurnPhase || ""] || (chatBusy ? "Working" : "");
  const hasHistory = chatLines.length > 0;

  const scrollToBottom = () => {
    const el = scrollRef.current;
    if (!el) return;
    el.scrollTo({ top: el.scrollHeight, behavior: "smooth" });
    setStickBottom(true);
  };

  // Mobile context drawer: open via the floating ctx-fab; the panel renders
  // as an overlay. We also flip the server-side flag so a snapshot refresh
  // doesn't yank it away mid-session.
  const openMobileCtx = () => {
    setMobileCtxOpen(true);
    void apiPost("/api/chat/context", { visible: true });
  };
  const closeMobileCtx = () => setMobileCtxOpen(false);

  // On desktop, the panel is an inline grid column driven by contextVisible.
  // On mobile, we render it as a drawer when mobileCtxOpen (and hide the
  // inline column via the responsive CSS).
  const showContextPanel = contextVisible;
  const mobileDrawerOpen = isMobile && mobileCtxOpen && contextVisible;

  return (
    <div className="chat-shell">
      <div className={`chat-layout ${showContextPanel ? "" : "no-context"}`}>
        <div className="messages-pane">
          <div className="messages-header">
            <span className="messages-title">Messages</span>
            {autoApprove && (
              <span
                className="auto-approve-badge"
                title="Mutating GitHub and MCP tools run without confirmation"
              >
                Auto-approve ON — mutating tools run without confirmation
              </span>
            )}
            <SessionPicker />
            {countLabel && (
              <span className="messages-count">{countLabel}</span>
            )}
            <div className="messages-header-actions">
              <button
                type="button"
                className={`btn-header-action btn-header-export${hasHistory ? "" : " hidden"}`}
                onClick={() => void apiFetchDownload("/api/chat/export")}
                title="Export transcript"
              >
                Export
              </button>
              <button
                type="button"
                className={`btn-header-action btn-header-clear${!hasHistory || chatBusy ? " hidden" : ""}`}
                onClick={() => void apiPost("/api/chat/clear")}
                title="Clear session"
              >
                Clear
              </button>
              {!contextVisible && (
                <button
                  type="button"
                  className="btn-header-action btn-header-ctx"
                  onClick={() =>
                    void apiPost("/api/chat/context", { visible: true })
                  }
                  title="Show context panel"
                >
                  Context
                </button>
              )}
              {chatBusy && phaseLabel && (
                <span
                  className={`messages-live phase-${chatTurnPhase || "model"}`}
                >
                  <span className="live-dot" aria-hidden="true" />
                  {phaseLabel}
                </span>
              )}
            </div>
          </div>
          <div ref={scrollRef} className="messages">
            <div className="msg-history">
              <ChatHistory
                blocks={blocks}
                scrollRef={scrollRef}
                onStickBottomChange={setStickBottom}
              />
            </div>
            <LiveDivider visible={liveActive} />
            <LiveZone />
          </div>
          <button
            type="button"
            onClick={scrollToBottom}
            className={`scroll-fab${stickBottom ? " hidden" : ""}`}
            aria-label="Scroll to bottom"
          >
            ↓ Bottom
          </button>
          {/* Mobile-only floating button to reopen the context panel as a
           * drawer. Hidden on desktop (CSS .ctx-fab display:none) and when
           * the drawer is already open. */}
          <button
            type="button"
            onClick={openMobileCtx}
            className={`ctx-fab${isMobile && !mobileDrawerOpen ? "" : " hidden"}`}
            aria-label="Open context panel"
            title="Open context panel"
          >
            <PanelRightOpen size={14} aria-hidden="true" />
            Context
          </button>
        </div>
        {showContextPanel && (
          <ContextPanel mobileOpen={mobileDrawerOpen} onMobileClose={closeMobileCtx} />
        )}
      </div>
      <ChatInput busy={chatBusy} />
    </div>
  );
}

const PHASE_LABELS: Record<string, string> = {
  model: "Thinking",
  tool: "Running tool",
  streaming: "Writing reply",
  reasoning: "Reasoning",
  summarizing: "Summarizing context",
  activity: "Loading skills",
};

async function apiFetchDownload(url: string) {
  try {
    const res = await fetch(url);
    if (!res.ok) return;
    const text = await res.text();
    const blob = new Blob([text], { type: "text/markdown" });
    const u = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = u;
    a.download = "chat-transcript.md";
    a.click();
    URL.revokeObjectURL(u);
  } catch {
    /* ignore */
  }
}

function ChatInput({ busy }: { busy: boolean }) {
  const [draft, setDraft] = useState("");
  const taRef = useRef<HTMLTextAreaElement>(null);

  const send = () => {
    const msg = draft.trim();
    if (!msg || busy) return;
    void apiPost("/api/chat", { message: msg });
    setDraft("");
    if (taRef.current) taRef.current.style.height = "auto";
  };

  const onKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    // Enter sends, Shift+Enter inserts a newline. Cmd/Ctrl+Enter also sends
    // (useful for IME composition where plain Enter is needed for confirm).
    if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
      e.preventDefault();
      send();
    }
  };

  const onInput = (e: React.ChangeEvent<HTMLTextAreaElement>) => {
    setDraft(e.target.value);
    const ta = e.target;
    ta.style.height = "auto";
    ta.style.height = `${Math.min(160, Math.max(42, ta.scrollHeight))}px`;
  };

  return (
    <div className="chat-input">
      <textarea
        ref={taRef}
        data-chat-input
        value={draft}
        onChange={onInput}
        onKeyDown={onKeyDown}
        disabled={busy}
        placeholder={
          busy
            ? "Waiting for model…"
            : "Message… (Enter send · Shift+Enter newline · /help · /clear · /new)"
        }
        rows={1}
      />
      <button
        type="button"
        className="btn btn-primary"
        onClick={send}
        disabled={busy || !draft.trim()}
      >
        Send
      </button>
      <button
        type="button"
        className="btn btn-ghost btn-cancel"
        onClick={() => void apiPost("/api/chat/cancel")}
        disabled={!busy}
      >
        Cancel
      </button>
    </div>
  );
}
