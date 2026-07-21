import { useEffect, useMemo, useRef, useState } from "react";
import { useStore } from "../../store/wsStore";
import { apiPost } from "../../lib/api";
import EmptyState from "../../components/EmptyState";
import ChatHistory from "./ChatHistory";
import ContextPanel from "./ContextPanel";
import SessionPicker from "./SessionPicker";
import { useChatUiStore } from "../../store/chatUiStore";
import ContextWindowMeter from "../../components/ContextWindowMeter";
import { PanelRightOpen, Search, Download, Trash2, ChevronUp, ChevronDown, ArrowDown, MessageSquareOff, MessageSquare, MessageCircleQuestion, Rows3 } from "lucide-react";
import { HistoryCoverageGap } from "./TranscriptMarkers";
import {
  buildChatBlocks,
  groupIntoTurns,
  trimInFlightHistoryItems,
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
  const chatOlderAvailable = useStore((s) => s.chat_older_available);
  const chatLinesTruncated = useStore((s) => s.chat_lines_truncated);
  const chatOlderInStore = useStore((s) => s.chat_older_in_store);
  const chatHistoryRevision = useStore((s) => s.chat_history_revision);
  const autoApprove = useStore((s) => s.auto_approve_mutations);
  const awaitingUserAnswer = useStore((s) => !!s.chat_pending_user_question);

  const [loadingOlder, setLoadingOlder] = useState(false);
  const scrollRef = useRef<HTMLDivElement>(null);

  const loadOlderMessages = async () => {
    if (loadingOlder || chatBusy) return;
    const el = scrollRef.current;
    const anchorKey =
      el?.querySelector<HTMLElement>("[data-block-key]")?.getAttribute("data-block-key") || "";
    const prevHeight = el?.scrollHeight ?? 0;
    const prevTop = el?.scrollTop ?? 0;
    setLoadingOlder(true);
    try {
      await apiPost("/api/chat/load-older");
      requestAnimationFrame(() => {
        const scroller = scrollRef.current;
        if (!scroller) return;
        const delta = scroller.scrollHeight - prevHeight;
        if (delta > 0) {
          scroller.scrollTop = prevTop + delta;
          return;
        }
        if (anchorKey) {
          const node = scroller.querySelector<HTMLElement>(
            `[data-block-key="${CSS.escape(anchorKey)}"]`,
          );
          node?.scrollIntoView({ block: "start" });
        }
      });
    } finally {
      setLoadingOlder(false);
    }
  };
  const [stickBottom, setStickBottom] = useState(true);
  const [newMessageBoundaryKey, setNewMessageBoundaryKey] = useState<string | null>(null);
  const prevItemCountRef = useRef(0);
  const isMobile = useIsMobile();
  const userMessageStyle = useChatUiStore((s) => s.userMessageStyle);
  const toggleUserMessageStyle = useChatUiStore((s) => s.toggleUserMessageStyle);
  const desktopCompactTranscript = useChatUiStore((s) => s.desktopCompactTranscript);
  const toggleDesktopCompactTranscript = useChatUiStore((s) => s.toggleDesktopCompactTranscript);

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
  }, [chatLines, outputs, reasoningOriginals]);
  const items = useMemo(() => {
    const grouped = groupIntoTurns(blocks);
    return trimInFlightHistoryItems(grouped, chatBusy);
  }, [blocks, chatBusy]);
  const stats = useMemo(() => messageStatsFromBlocks(blocks), [blocks]);
  const countLabel = formatMessageCount(stats);

  const itemKey = (item: (typeof items)[number]) =>
    item.type === "block" ? item.block.key : item.turn.key;

  const searchTextForItem = (item: (typeof items)[number]): string => {
    if (item.type === "block") {
      const b = item.block;
      const parts = [b.message?.body || b.reasoningText || ""];
      if (b.type === "tool-group" && b.group) {
        parts.push(b.group.toolName, b.group.args || "");
        for (const s of b.group.steps) {
          if (s.output) parts.push(s.output);
        }
      }
      return parts.join("\n");
    }
    const parts: string[] = [];
    if (item.turn.user?.message?.body) parts.push(item.turn.user.message.body);
    for (const b of item.turn.process) {
      parts.push(b.message?.body || b.reasoningText || "");
      if (b.type === "tool-group" && b.group) {
        parts.push(b.group.toolName, b.group.args || "");
        for (const s of b.group.steps) {
          if (s.output) parts.push(s.output);
        }
      }
    }
    if (item.turn.answer?.message?.body) parts.push(item.turn.answer.message.body);
    return parts.join("\n");
  };

  // Ordered list of item keys that match the search query.
  const searchMatchKeys = useMemo(() => {
    if (!searchQuery.trim()) return [];
    const q = searchQuery.toLowerCase();
    return items
      .filter((item) => searchTextForItem(item).toLowerCase().includes(q))
      .map(itemKey);
  }, [items, searchQuery]);

  const activeMatchKey = searchMatchKeys[activeMatchIdx] || "";

  useEffect(() => {
    const prev = prevItemCountRef.current;
    if (items.length > prev && !stickBottom && items[prev]) {
      setNewMessageBoundaryKey(itemKey(items[prev]));
    }
    if (stickBottom) setNewMessageBoundaryKey(null);
    prevItemCountRef.current = items.length;
  }, [items, stickBottom, chatHistoryRevision]);

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
    <div
      className={`chat-shell${desktopCompactTranscript ? " is-compact-transcript" : ""}`}
    >
      <div className={`chat-layout ${showContextPanel ? "" : "no-context"}`}>
        <div className="messages-pane">
          <div className="messages-header">
            <span className="messages-title">消息</span>
            {autoApprove && (
              <span
                className="auto-approve-badge"
                title="变更类 GitHub / MCP 工具将跳过确认直接执行"
              >
                自动批准
              </span>
            )}
            <SessionPicker />
            {countLabel && (
              <span className="messages-count">{countLabel}</span>
            )}
            <ContextWindowMeter className="messages-ctx-meter" />
            <div className="messages-header-actions">
              <button
                type="button"
                className={`btn-header-action btn-header-layout${desktopCompactTranscript ? " is-active" : ""}`}
                onClick={toggleDesktopCompactTranscript}
                title={
                  desktopCompactTranscript
                    ? "紧凑 transcript（点击恢复标准布局）"
                    : "标准 transcript（点击切换紧凑模式）"
                }
                aria-pressed={desktopCompactTranscript}
              >
                <Rows3 size={14} className="btn-header-icon" />
                <span className="btn-header-label">
                  {desktopCompactTranscript ? "紧凑" : "标准"}
                </span>
              </button>
              <button
                type="button"
                className={`btn-header-action btn-header-layout${userMessageStyle === "bubble" ? " is-active" : ""}`}
                onClick={toggleUserMessageStyle}
                title={
                  userMessageStyle === "bubble"
                    ? "用户消息：气泡模式（点击切换为平铺）"
                    : "用户消息：平铺模式（点击切换为气泡）"
                }
                aria-pressed={userMessageStyle === "bubble"}
              >
                <MessageSquare size={14} className="btn-header-icon" />
                <span className="btn-header-label">
                  {userMessageStyle === "bubble" ? "气泡" : "平铺"}
                </span>
              </button>
              <button
                type="button"
                className="btn-header-action btn-header-search"
                onClick={() => setSearchOpen((v) => !v)}
                title="搜索对话（Ctrl/Cmd+F）"
              >
                <Search size={14} className="btn-header-icon" />
                <span className="btn-header-label">搜索</span>
              </button>
              <button
                type="button"
                className={`btn-header-action btn-header-export${hasHistory ? "" : " hidden"}`}
                onClick={() => void apiFetchDownload("/api/chat/export")}
                title="导出记录"
              >
                <Download size={14} className="btn-header-icon" />
                <span className="btn-header-label">导出</span>
              </button>
              <button
                type="button"
                className={`btn-header-action btn-header-clear${!hasHistory || chatBusy ? " hidden" : ""}`}
                onClick={() => void apiPost("/api/chat/clear")}
                title="清空会话"
              >
                <Trash2 size={14} className="btn-header-icon" />
                <span className="btn-header-label">清空</span>
              </button>
              {!contextVisible && (
                <button
                  type="button"
                  className="btn-header-action btn-header-ctx"
                  onClick={() =>
                    void apiPost("/api/chat/context", { visible: true })
                  }
                  title="显示上下文面板"
                >
                  上下文
                </button>
              )}
            </div>
          </div>
          {searchOpen && (
            <div className="chat-search-bar">
              <input
                type="text"
                className="chat-search-input"
                placeholder="搜索对话…"
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
                    aria-label="上一个匹配"
                    title="上一个（Shift+Enter）"
                  >
                    <ChevronUp size={14} />
                  </button>
                  <button
                    type="button"
                    className="chat-search-nav"
                    onClick={nextMatch}
                    aria-label="下一个匹配"
                    title="下一个（Enter）"
                  >
                    <ChevronDown size={14} />
                  </button>
                </>
              )}
              {searchQuery && searchMatchKeys.length === 0 && (
                <span className="chat-search-count">无匹配</span>
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
              {chatOlderAvailable && !chatBusy && (
                <div className="chat-load-older-wrap">
                  {(chatOlderInStore || chatLinesTruncated) && (
                    <HistoryCoverageGap
                      variant={chatLinesTruncated && !chatOlderInStore ? "memory" : "store"}
                    />
                  )}
                  <button
                    type="button"
                    className="chat-load-older-btn chat-load-older-center"
                    disabled={loadingOlder}
                    onClick={() => void loadOlderMessages()}
                  >
                    {loadingOlder ? "加载中…" : "加载更早消息"}
                  </button>
                </div>
              )}
              <ChatHistory
                items={items}
                scrollRef={scrollRef}
                stickBottom={stickBottom}
                onStickBottomChange={setStickBottom}
                searchQuery={searchQuery}
                activeMatchKey={activeMatchKey}
                newMessageBoundaryKey={newMessageBoundaryKey}
              />
            </div>
          </div>
          <button
            type="button"
            onClick={scrollToBottom}
            className={`scroll-fab${stickBottom ? " hidden" : ""}`}
            aria-label="回到底部"
            title="回到底部"
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
            aria-label="打开上下文面板"
            title="打开上下文面板"
          >
            <PanelRightOpen size={14} aria-hidden="true" />
            上下文
          </button>
        </div>
        {showContextPanel && (
          <ContextPanel mobileOpen={mobileDrawerOpen} onMobileClose={closeMobileCtx} />
        )}
      </div>
      <AskUserBanner />
      {/* While ask_user is pending, answers go through the banner (options + custom). */}
      {awaitingUserAnswer && !chatBusy ? null : <ChatInput busy={chatBusy} />}
    </div>
  );
}

function AskUserBanner() {
  const pending = useStore((s) => s.chat_pending_user_question);
  const chatBusy = useStore((s) => s.chat_busy);
  const [custom, setCustom] = useState("");
  const [sending, setSending] = useState(false);
  const customRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    setCustom("");
    setSending(false);
    if (pending) {
      requestAnimationFrame(() => customRef.current?.focus());
    }
  }, [pending?.id]);

  if (!pending || chatBusy) return null;

  const submit = (answer: string) => {
    const msg = answer.trim();
    if (!msg || sending) return;
    setSending(true);
    void apiPost("/api/chat", { message: msg }).finally(() => setSending(false));
    setCustom("");
  };

  const hasOptions = pending.options.length > 0;

  return (
    <div className="ask-user-banner" role="region" aria-label="助手提问">
      <div className="ask-user-banner-inner">
        <div className="ask-user-card">
          <div className="ask-user-card-head">
            <span className="ask-user-card-icon" aria-hidden="true">
              <MessageCircleQuestion size={18} strokeWidth={2} />
            </span>
            <div className="ask-user-card-head-text">
              <div className="ask-user-banner-label">需要你的回答</div>
              <div className="ask-user-banner-question">{pending.question}</div>
            </div>
          </div>

          {pending.context ? (
            <div className="ask-user-banner-context">{pending.context}</div>
          ) : null}

          {hasOptions ? (
            <div className="ask-user-banner-options" role="group" aria-label="建议回答">
              {pending.options.map((opt, i) => (
                <button
                  key={opt}
                  type="button"
                  className="ask-user-option"
                  disabled={sending}
                  onClick={() => submit(opt)}
                >
                  <span className="ask-user-option-idx" aria-hidden="true">
                    {i + 1}
                  </span>
                  <span className="ask-user-option-text">{opt}</span>
                </button>
              ))}
            </div>
          ) : null}

          <form
            className="ask-user-custom"
            onSubmit={(e) => {
              e.preventDefault();
              submit(custom);
            }}
          >
            <input
              ref={customRef}
              type="text"
              className="ask-user-custom-input"
              value={custom}
              onChange={(e) => setCustom(e.target.value)}
              disabled={sending}
              placeholder={hasOptions ? "或输入自定义回答…" : "输入你的回答…"}
              aria-label="自定义回答"
            />
            <button
              type="submit"
              className="btn btn-primary ask-user-custom-send"
              disabled={sending || !custom.trim()}
            >
              发送
            </button>
          </form>
        </div>
      </div>
    </div>
  );
}

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
      <div className="chat-input-inner">
        <textarea
          ref={taRef}
          data-chat-input
          value={draft}
          onChange={onInput}
          onKeyDown={onKeyDown}
          disabled={busy}
          placeholder={
            busy
              ? "等待模型回复…"
              : "输入消息…（Enter 换行 · Shift+Enter 发送 · /help）"
          }
          rows={1}
        />
        <button
          type="button"
          className="btn btn-primary"
          onClick={send}
          disabled={busy || !draft.trim()}
        >
          发送
        </button>
        {busy && (
          <button
            type="button"
            className="btn btn-ghost btn-cancel"
            onClick={() => void apiPost("/api/chat/cancel")}
          >
            取消
          </button>
        )}
      </div>
    </div>
  );
}
