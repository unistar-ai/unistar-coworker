import { useEffect, useMemo, useRef, useState } from "react";
import { useStore } from "../../store/wsStore";
import { apiPost } from "../../lib/api";
import EmptyState from "../../components/EmptyState";
import ChatHistory from "./ChatHistory";
import LiveZone, { useLiveZoneActive } from "./LiveZone";
import LiveDivider from "./LiveDivider";
import ContextPanel from "./ContextPanel";
import SessionPicker from "./SessionPicker";
import { PanelRightOpen, Search, Download, Trash2, ChevronUp, ChevronDown, ArrowDown, MessageSquareOff } from "lucide-react";
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
  const reasoningOriginals = useStore((s) => s.chat_reasoning_originals);
  const autoApprove = useStore((s) => s.auto_approve_mutations);
  const chatTurnPhase = useStore((s) => s.chat_turn_phase);

  const scrollRef = useRef<HTMLDivElement>(null);
  const [stickBottom, setStickBottom] = useState(true);
  const liveActive = useLiveZoneActive();
  const isMobile = useIsMobile();

  // Esc cancels generation when busy.
  useEffect(() => {
    if (!chatBusy) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        void apiPost("/api/chat/cancel");
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [chatBusy]);

  // Ctrl/Cmd+F opens in-chat search.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key === "f") {
        const target = e.target as HTMLElement;
        // Don't intercept browser's native find when already in a text field.
        if (target.tagName === "INPUT" || target.tagName === "TEXTAREA") return;
        e.preventDefault();
        setSearchOpen(true);
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, []);
  // Local drawer state for the mobile context panel. The server-side
  // chat_context_visible flag drives the desktop column; on mobile we layer a
  // local "drawer open" toggle on top so the panel can be shown/hidden as an
  // overlay without round-tripping through the backend for every toggle.
  const [mobileCtxOpen, setMobileCtxOpen] = useState(false);
  const [searchQuery, setSearchQuery] = useState("");
  const [searchOpen, setSearchOpen] = useState(false);
  const [activeMatchIdx, setActiveMatchIdx] = useState(0);

  const blocks = useMemo(() => {
    const built = buildChatBlocks(chatLines, outputs, reasoningOriginals);
    // Mark the last assistant message block for the Regenerate button.
    for (let i = built.length - 1; i >= 0; i--) {
      if (built[i].type === "message" && built[i].message?.role === "assistant") {
        built[i].isLastAssistant = true;
        break;
      }
    }
    return built;
  }, [chatLines, outputs]);
  const stats = useMemo(() => messageStatsFromBlocks(blocks), [blocks]);
  const countLabel = formatMessageCount(stats);

  // Ordered list of block keys that match the search query.
  const searchMatchKeys = useMemo(() => {
    if (!searchQuery.trim()) return [];
    const q = searchQuery.toLowerCase();
    return blocks.filter((b) => {
      const text = b.message?.body || b.reasoningText || "";
      return text.toLowerCase().includes(q);
    }).map((b) => b.key);
  }, [blocks, searchQuery]);

  const activeMatchKey = searchMatchKeys[activeMatchIdx] || "";

  const gotoMatch = (idx: number) => {
    if (searchMatchKeys.length === 0) return;
    const next = ((idx % searchMatchKeys.length) + searchMatchKeys.length) % searchMatchKeys.length;
    setActiveMatchIdx(next);
  };
  const nextMatch = () => gotoMatch(activeMatchIdx + 1);
  const prevMatch = () => gotoMatch(activeMatchIdx - 1);

  if (!chatEnabled) {
    return (
      <div className="chat-shell">
        <EmptyState
          icon={MessageSquareOff}
          title="Chat is disabled"
          description="Enable chat in your config file to start conversing with the agent."
        />
      </div>
    );
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
                className="btn-header-action btn-header-search"
                onClick={() => setSearchOpen((v) => !v)}
                title="Search in chat (Ctrl/Cmd+F)"
              >
                <Search size={14} className="btn-header-icon" />
                <span className="btn-header-label">Search</span>
              </button>
              <button
                type="button"
                className={`btn-header-action btn-header-export${hasHistory ? "" : " hidden"}`}
                onClick={() => void apiFetchDownload("/api/chat/export")}
                title="Export transcript"
              >
                <Download size={14} className="btn-header-icon" />
                <span className="btn-header-label">Export</span>
              </button>
              <button
                type="button"
                className={`btn-header-action btn-header-clear${!hasHistory || chatBusy ? " hidden" : ""}`}
                onClick={() => void apiPost("/api/chat/clear")}
                title="Clear session"
              >
                <Trash2 size={14} className="btn-header-icon" />
                <span className="btn-header-label">Clear</span>
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
          {searchOpen && (
            <div className="chat-search-bar">
              <input
                type="text"
                className="chat-search-input"
                placeholder="Search in chat…"
                value={searchQuery}
                autoFocus
                onChange={(e) => {
                  setSearchQuery(e.target.value);
                  setActiveMatchIdx(0);
                }}
                onKeyDown={(e) => {
                  if (e.key === "Escape") {
                    setSearchOpen(false);
                    setSearchQuery("");
                  } else if (e.key === "Enter") {
                    e.preventDefault();
                    if (e.shiftKey) prevMatch();
                    else nextMatch();
                  }
                }}
              />
              {searchQuery && searchMatchKeys.length > 0 && (
                <>
                  <span className="chat-search-count">
                    {activeMatchIdx + 1} / {searchMatchKeys.length}
                  </span>
                  <button
                    type="button"
                    className="chat-search-nav"
                    onClick={prevMatch}
                    aria-label="Previous match"
                    title="Previous (Shift+Enter)"
                  >
                    <ChevronUp size={14} />
                  </button>
                  <button
                    type="button"
                    className="chat-search-nav"
                    onClick={nextMatch}
                    aria-label="Next match"
                    title="Next (Enter)"
                  >
                    <ChevronDown size={14} />
                  </button>
                </>
              )}
              {searchQuery && searchMatchKeys.length === 0 && (
                <span className="chat-search-count">No matches</span>
              )}
              <button
                type="button"
                className="chat-search-close"
                onClick={() => {
                  setSearchOpen(false);
                  setSearchQuery("");
                }}
              >
                ×
              </button>
            </div>
          )}
          <div ref={scrollRef} className="messages">
            <div className="msg-history">
              <ChatHistory
                blocks={blocks}
                scrollRef={scrollRef}
                stickBottom={stickBottom}
                onStickBottomChange={setStickBottom}
                searchQuery={searchQuery}
                activeMatchKey={activeMatchKey}
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
            title="Scroll to bottom"
          >
            <ArrowDown size={16} />
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
  const draft = useStore((s) => s.chatDraft);
  const setDraft = useStore((s) => s.setChatDraft);
  const taRef = useRef<HTMLTextAreaElement>(null);
  const historyRef = useRef<string[]>([]);
  const historyIdxRef = useRef(-1);

  const send = () => {
    const msg = draft.trim();
    if (!msg || busy) return;
    // Push to input history (dedup consecutive).
    const hist = historyRef.current;
    if (hist[hist.length - 1] !== msg) {
      hist.push(msg);
      if (hist.length > 100) hist.shift();
    }
    historyIdxRef.current = -1;
    void apiPost("/api/chat", { message: msg });
    setDraft("");
    if (taRef.current) taRef.current.style.height = "auto";
  };

  const onKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    // Enter inserts a newline; Shift+Enter sends.
    if (e.key === "Enter" && e.shiftKey && !e.nativeEvent.isComposing) {
      e.preventDefault();
      send();
      return;
    }
    // ↑/↓ navigate input history when cursor is on first/last line.
    const hist = historyRef.current;
    if (hist.length === 0) return;
    if (e.key === "ArrowUp" && e.currentTarget.selectionStart === 0) {
      e.preventDefault();
      const idx = historyIdxRef.current === -1
        ? hist.length - 1
        : Math.max(0, historyIdxRef.current - 1);
      historyIdxRef.current = idx;
      setDraft(hist[idx]);
    } else if (e.key === "ArrowDown" && e.currentTarget.selectionStart === draft.length) {
      e.preventDefault();
      const idx = historyIdxRef.current;
      if (idx === -1) return;
      if (idx >= hist.length - 1) {
        historyIdxRef.current = -1;
        setDraft("");
      } else {
        historyIdxRef.current = idx + 1;
        setDraft(hist[idx + 1]);
      }
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
            : "Message… (Enter newline · Shift+Enter send · /help). GitHub: owner/repo or PR URL"
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
