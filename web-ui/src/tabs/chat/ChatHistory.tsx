import { useMemo, useRef, useState, useEffect } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { MessageSquare, ChevronRight } from "lucide-react";
import Markdown from "../../components/Markdown";
import ReasoningCard from "../../components/ReasoningCard";
import EmptyState from "../../components/EmptyState";
import { useStore } from "../../store/wsStore";
import { toolMeta, normalizeReasoningText, reasoningHasDistinctOriginal, isInFlightUserTurn } from "./parser";
import { ToolStepOutput } from "./toolOutput";
import ToolDetailBody from "./ToolDetailBody";
import type {
  ChatBlock,
  ChatMessage,
  ChatHistoryItem,
  ToolStep,
  ToolGroup,
  ToolStepKind,
} from "./parser";
import AgentTurnCard, {
  type BlockRendererProps,
  TurnMessageBody,
  UserTurnView,
  ChatSearchQueryContext,
  AgentMessageActions,
} from "./AgentTurnCard";
import MessageTurnFrame from "./MessageTurnFrame";
import LiveZone from "./LiveZone";
import {
  pickToolRowSubtitle,
  toolCollapsedSummary,
  toolRowTitle,
} from "./toolDisplay";
import ErrorMessageBody from "./ErrorMessageBody";
import {
  estimateItemSize,
  isNewUserTurn,
  itemKey,
  scrollItemIntoView,
  VIRTUAL_OVERSCAN,
  VIRTUAL_THRESHOLD,
} from "./chatScroll";
import { useChatScrollOrchestrator } from "./useChatScrollOrchestrator";

export default function ChatHistory({
  items,
  scrollRef,
  stickBottom,
  onStickBottomChange,
  searchQuery = "",
  activeMatchKey = "",
}: {
  items: ChatHistoryItem[];
  scrollRef: React.RefObject<HTMLDivElement | null>;
  stickBottom: boolean;
  onStickBottomChange?: (stick: boolean) => void;
  searchQuery?: string;
  activeMatchKey?: string;
}) {
  const chatBusy = useStore((s) => s.chat_busy);
  const chatStreaming = useStore((s) => s.chat_streaming);
  const chatTurnParts = useStore((s) => s.chat_turn_parts);
  const mcpServers = useStore((s) => s.mcp_servers);
  const prevCountRef = useRef(0);
  const {
    spacerPx,
    pinTo,
    releasePin,
    pinnedKeyRef,
    userScrollRef,
    notifyContentChange,
    scrollToBottom,
  } = useChatScrollOrchestrator({ scrollRef, stickBottom, onStickBottomChange });

  const mcpPrefixes = useMemo(
    () => mcpServers.map((s) => ({ id: s.id, prefix: s.prefix || `${s.id}_` })),
    [mcpServers],
  );

  // Track which block keys existed on the previous render so we can flag newly
  // appended blocks with an entrance animation. Only newly-appended (not
  // historically replayed) blocks get the .is-new class.
  const renderBlock = (props: BlockRendererProps) => (
    <BlockRenderer {...props} />
  );

  const prevKeysRef = useRef<Set<string>>(new Set());
  const newKeys = useMemo(() => {
    const prev = prevKeysRef.current;
    const next = new Set(items.map(itemKey));
    const added = new Set<string>();
    for (const k of next) {
      if (!prev.has(k)) added.add(k);
    }
    prevKeysRef.current = next;
    return added;
  }, [items]);

  const searchTextForItem = (item: ChatHistoryItem): string => {
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

  const searchMatches = useMemo(() => {
    if (!searchQuery.trim()) return new Set<string>();
    const q = searchQuery.toLowerCase();
    const matches = new Set<string>();
    for (const item of items) {
      if (searchTextForItem(item).toLowerCase().includes(q)) {
        matches.add(itemKey(item));
      }
    }
    return matches;
  }, [items, searchQuery]);

  const virtualizer = useVirtualizer({
    count: items.length,
    getScrollElement: () => scrollRef.current,
    getItemKey: (i) => itemKey(items[i]),
    estimateSize: (i) => estimateItemSize(items[i]),
    overscan: VIRTUAL_OVERSCAN,
  });

  useEffect(() => {
    if (!chatBusy) releasePin();
  }, [chatBusy, releasePin]);

  const count = items.length;
  useEffect(() => {
    if (count <= prevCountRef.current) {
      prevCountRef.current = count;
      return;
    }
    const lastItem = items[count - 1];
    const lastKey = itemKey(lastItem);
    if (isNewUserTurn(lastItem, newKeys.has(lastKey))) {
      userScrollRef.current = false;
      onStickBottomChange?.(false);
      const scrollVirtual = (key: string) => {
        const idx = items.findIndex((item) => itemKey(item) === key);
        if (idx >= 0) virtualizer.scrollToIndex(idx, { align: "start" });
      };
      pinTo(lastKey, items.length >= VIRTUAL_THRESHOLD ? scrollVirtual : undefined);
    } else if (stickBottom && !pinnedKeyRef.current) {
      if (items.length >= VIRTUAL_THRESHOLD) {
        virtualizer.scrollToIndex(count - 1, { align: "end" });
      } else {
        scrollItemIntoView(lastKey, "end");
      }
      scrollToBottom("instant");
    }
    prevCountRef.current = count;
  }, [count, items, stickBottom, newKeys, virtualizer, onStickBottomChange, pinTo, pinnedKeyRef, scrollToBottom]);

  useEffect(() => {
    if (pinnedKeyRef.current) return;
    if (!stickBottom) return;
    notifyContentChange();
  }, [
    chatBusy,
    chatStreaming,
    chatTurnParts,
    stickBottom,
    notifyContentChange,
    pinnedKeyRef,
    items.length,
  ]);

  // Scroll to the active search match when it changes.
  useEffect(() => {
    if (!activeMatchKey) return;
    const idx = items.findIndex((item) => itemKey(item) === activeMatchKey);
    if (idx < 0) return;
    const reduceMotion =
      typeof window !== "undefined" &&
      window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    const behavior: ScrollBehavior = reduceMotion ? "auto" : "smooth";
    if (items.length >= VIRTUAL_THRESHOLD) {
      virtualizer.scrollToIndex(idx, { align: "center", behavior });
    } else {
      const el = document.querySelector(`[data-block-key="${activeMatchKey}"]`);
      el?.scrollIntoView({ behavior, block: "center" });
    }
  }, [activeMatchKey]); // eslint-disable-line react-hooks/exhaustive-deps

  if (items.length === 0) {
    if (chatBusy) {
      return (
        <div className="chat-block-list">
          <div className="chat-turn">
            <LiveZone />
          </div>
        </div>
      );
    }
    return (
      <EmptyState
        icon={MessageSquare}
        title="No messages yet"
        description="Ask about your workspace — code, tests, or docs. For GitHub, the agent will ask which repo if you have not named one yet."
      />
    );
  }

  const liveNestedInLast = chatBusy && isInFlightUserTurn(items[items.length - 1]);

  // Short sessions: render all blocks directly (no virtualizer). This avoids
  // the estimate/measure cycle that causes blank gaps and scroll jumps when
  // markdown blocks have variable height. Virtualization only kicks in for
  // long sessions (>= VIRTUAL_THRESHOLD blocks) where DOM size matters.
  // NOTE: use .chat-block-list (not .messages) so we don't create a second
  // scrolling/padding container — the outer .messages in ChatTab already is
  // the scroll container with padding.
  if (items.length < VIRTUAL_THRESHOLD) {
    return (
      <ChatSearchQueryContext.Provider value={searchQuery}>
        <div className="chat-block-list">
          {items.map((item, index) => {
            const key = itemKey(item);
            const liveTail = liveNestedInLast && index === items.length - 1;
            return (
              <div
                key={key}
                data-block-key={key}
                className={`${itemWrapClass(item)}${newKeys.has(key) ? " is-new" : ""}${searchMatches.has(key) ? " is-search-match" : ""}${activeMatchKey === key ? " is-search-active" : ""}`}
              >
                <HistoryItemView
                  item={item}
                  mcpPrefixes={mcpPrefixes}
                  renderBlock={renderBlock}
                  liveTail={liveTail}
                />
              </div>
            );
          })}
          {chatBusy && !liveNestedInLast && (
            <div className="chat-turn">
              <LiveZone />
            </div>
          )}
        </div>
        {spacerPx > 0 && (
          <div className="chat-scroll-pin-spacer" style={{ height: spacerPx }} aria-hidden="true" />
        )}
      </ChatSearchQueryContext.Provider>
    );
  }

  return (
    <ChatSearchQueryContext.Provider value={searchQuery}>
      <div
        style={{
          height: `${virtualizer.getTotalSize()}px`,
          width: "100%",
          position: "relative",
        }}
      >
          {virtualizer.getVirtualItems().map((vi) => {
            const item = items[vi.index];
            const key = itemKey(item);
            const liveTail = liveNestedInLast && vi.index === items.length - 1;
            return (
              <div
                key={key}
                data-index={vi.index}
                ref={virtualizer.measureElement}
                style={{
                  position: "absolute",
                  top: 0,
                  left: 0,
                  width: "100%",
                  display: "flex",
                  flexDirection: "column",
                  transform: `translateY(${vi.start}px)`,
                }}
              >
                <div data-block-key={key} className={`${itemWrapClass(item)}${newKeys.has(key) ? " is-new" : ""}${searchMatches.has(key) ? " is-search-match" : ""}${activeMatchKey === key ? " is-search-active" : ""}`}>
                  <HistoryItemView
                    item={item}
                    mcpPrefixes={mcpPrefixes}
                    renderBlock={renderBlock}
                    liveTail={liveTail}
                  />
                </div>
              </div>
            );
          })}
      </div>
      {chatBusy && !liveNestedInLast && (
        <div className="chat-turn">
          <LiveZone />
        </div>
      )}
      {spacerPx > 0 && (
        <div className="chat-scroll-pin-spacer" style={{ height: spacerPx }} aria-hidden="true" />
      )}
    </ChatSearchQueryContext.Provider>
  );
}

function itemWrapClass(item: ChatHistoryItem): string {
  if (item.type === "block") return blockWrapClass(item.block);
  return "msg-block msg-block-turn";
}

function HistoryItemView({
  item,
  mcpPrefixes,
  renderBlock,
  liveTail = false,
}: {
  item: ChatHistoryItem;
  mcpPrefixes: { id: string; prefix: string }[];
  renderBlock: (props: BlockRendererProps) => React.ReactNode;
  /** Nest LiveZone in this turn so in-flight stream continues the same chat-turn. */
  liveTail?: boolean;
}) {
  if (item.type === "block") {
    return <BlockRenderer block={item.block} mcpPrefixes={mcpPrefixes} />;
  }
  const { turn } = item;
  return (
    <div className="chat-turn">
      {turn.user?.message && <UserTurnView msg={turn.user.message} />}
      <AgentTurnCard turn={turn} mcpPrefixes={mcpPrefixes} renderBlock={renderBlock} />
      {liveTail && <LiveZone />}
    </div>
  );
}

function blockWrapClass(block: ChatBlock): string {
  if (block.type === "message" && block.message) {
    const role = block.message.role;
    if (role === "you") return "msg-block msg-block-you";
    if (role === "assistant") return "msg-block msg-block-assistant";
    if (role === "error") return "msg-block msg-block-error";
    if (role === "system" || role === "meta") return "msg-block msg-block-system";
    return "msg-block msg-block-assistant";
  }
  if (block.type === "tool-batch") return "msg-block msg-block-tool-batch";
  if (block.type === "tool-group") return "msg-block msg-block-tool-group";
  if (block.type === "reasoning") return "msg-block msg-block-reasoning";
  return "msg-block";
}

export function BlockRenderer({
  block,
  mcpPrefixes,
  compact: _compact = false,
  hideLabel = false,
  inline = false,
}: BlockRendererProps) {
  if (block.type === "message" && block.message) {
    if (inline) {
      return <TurnMessageBody msg={block.message} nested />;
    }
    return (
      <MessageView
        msg={block.message}
        isLastAssistant={block.isLastAssistant}
        hideLabel={hideLabel}
      />
    );
  }
  if (block.type === "tool-batch") {
    return <ToolBatchView block={block} mcpPrefixes={mcpPrefixes} />;
  }
  if (block.type === "tool-group" && block.group) {
    if (inline) {
      return (
        <ToolDetailBody
          group={block.group}
          mcpPrefixes={mcpPrefixes}
          variant={inline ? "process" : "card"}
        />
      );
    }
    return (
      <ToolGroupBlockView
        group={block.group}
        mcpPrefixes={mcpPrefixes}
        defaultExpanded={false}
      />
    );
  }
  if (block.type === "reasoning") {
    if (inline) {
      const body = normalizeReasoningText(block.reasoningText || "");
      if (!body) return null;
      return (
        <div className="chat-process-inline-body">
          <Markdown variant="turn">{body}</Markdown>
        </div>
      );
    }
    return <ReasoningBlockView text={block.reasoningText || ""} original={block.reasoningOriginal} />;
  }
  return null;
}

function MessageView({
  msg,
  isLastAssistant,
  hideLabel = false,
}: {
  msg: ChatMessage;
  isLastAssistant?: boolean;
  hideLabel?: boolean;
}) {
  const timeIso = useStore((s) => s.chat_line_times[String(msg.lineIndex)]) || undefined;

  // System/meta messages get a centered pill style.
  if (msg.role === "system" || msg.role === "meta") {
    const cls = /cleared|^new session/i.test(msg.body)
      ? "msg-system system-session"
      : /approval|denied|approved/i.test(msg.body)
        ? "msg-system system-approval"
        : "msg-system";
    return <div className={cls}>{msg.body}</div>;
  }

  // Inline embeds (process detail) — body only.
  if (hideLabel) {
    return <TurnMessageBody msg={msg} nested />;
  }

  if (msg.role === "you") {
    return <UserTurnView msg={msg} />;
  }

  if (msg.role === "error") {
    return (
      <article className="chat-agent-turn chat-error-turn">
        <MessageTurnFrame role="error" name="错误" timeIso={timeIso}>
          <ErrorMessageBody body={msg.body} framed />
        </MessageTurnFrame>
      </article>
    );
  }

  return (
    <article className="chat-agent-turn">
      <MessageTurnFrame
        role="agent"
        name="助手"
        timeIso={timeIso}
        footer={<AgentMessageActions msg={msg} isLastAssistant={isLastAssistant} />}
        footerPinned={!!isLastAssistant}
      >
        <TurnMessageBody msg={msg} framed />
      </MessageTurnFrame>
    </article>
  );
}

/** Standalone reasoning block — collapsible, default collapsed. */
function ReasoningBlockView({ text, original }: { text: string; original?: string }) {
  const [expanded, setExpanded] = useState(false);
  return (
    <ReasoningCard
      text={text}
      original={original}
      expanded={expanded}
      onToggle={() => setExpanded((e) => !e)}
    />
  );
}

const STEP_ICON: Record<ToolStepKind, string> = {
  start: "→",
  done: "✓",
  "approval-pending": "⏳",
  approval: "⤴",
  warn: "⚠",
  reasoning: "💭",
  interim: "💬",
  meta: "·",
};

const STATUS_PILL: Record<string, string> = {
  ok: "完成",
  err: "失败",
  pending: "等待中",
  running: "执行中",
  warn: "警告",
};

function ToolStatusPill({ status }: { status: string }) {
  const label = STATUS_PILL[status];
  if (!label) return null;
  return <span className={`tool-status-pill status-${status}`}>{label}</span>;
}

function ToolBatchView({
  block,
  mcpPrefixes,
}: {
  block: ChatBlock;
  mcpPrefixes: { id: string; prefix: string }[];
}) {
  const groups = block.groups || [];
  const [expanded, setExpanded] = useState(false);

  const okCount = groups.filter((g) => g.status === "ok").length;
  const errCount = groups.filter((g) => g.status === "err").length;
  const labels = groups
    .map((g) => toolMeta(g.toolName, mcpPrefixes).label)
    .join(" · ");
  const truncate = (s: string, max: number) =>
    s.length > max ? s.slice(0, max - 1) + "…" : s;

  const openBatch = () => setExpanded(true);

  return (
    <div className={`tool-run-strip${expanded ? " is-expanded" : ""}`}>
      <button
        type="button"
        onClick={() => setExpanded((e) => !e)}
        className="tool-run-summary"
        aria-expanded={expanded}
      >
        <span className="tool-run-count">{groups.length} tools</span>
        <span className="tool-run-labels" title={labels}>
          {truncate(labels, 72)}
        </span>
        <span className="tool-run-stats">
          {okCount > 0 && <span className="ok">{okCount}✓</span>}
          {errCount > 0 && <span className="err">{errCount}✗</span>}
        </span>
        <span className="tool-run-chevron" aria-hidden="true">
          <ChevronRight size={12} />
        </span>
      </button>

      {!expanded && (
        <div className="tool-run-chips">
          {groups.map((g, i) => (
            <button
              key={i}
              type="button"
              onClick={openBatch}
              className={`tool-run-chip status-${g.status}`}
              title={g.args || g.toolName}
            >
              {toolMeta(g.toolName, mcpPrefixes).label}
            </button>
          ))}
        </div>
      )}

      {expanded && (
        <div className="tool-run-list">
          {groups.map((g, i) => (
            <div key={i} className={`tool-run-item status-${g.status}`}>
              <ToolGroupBlockView group={g} mcpPrefixes={mcpPrefixes} inBatch />
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function ToolGroupBlockView({
  group,
  mcpPrefixes,
  inBatch = false,
  defaultExpanded = false,
}: {
  group: ToolGroup;
  mcpPrefixes: { id: string; prefix: string }[];
  inBatch?: boolean;
  defaultExpanded?: boolean;
}) {
  const [expanded, setExpanded] = useState(defaultExpanded);
  const compact =
    !defaultExpanded &&
    group.status !== "running" &&
    group.status !== "pending" &&
    !expanded;

  if (compact) {
    return (
      <ToolCompactChip
        group={group}
        mcpPrefixes={mcpPrefixes}
        onExpand={() => setExpanded(true)}
      />
    );
  }

  return (
    <ToolGroupView
      group={group}
      expanded={expanded || defaultExpanded}
      onToggle={() => setExpanded((e) => !e)}
      mcpPrefixes={mcpPrefixes}
      inBatch={inBatch}
    />
  );
}

function ToolCompactChip({
  group,
  mcpPrefixes,
  onExpand,
}: {
  group: ToolGroup;
  mcpPrefixes: { id: string; prefix: string }[];
  onExpand: () => void;
}) {
  const meta = toolMeta(group.toolName, mcpPrefixes);
  const summary = toolCollapsedSummary(group);
  const subtitle = pickToolRowSubtitle(group);

  return (
    <div
      className={`tool-chip status-${group.status} clickable`}
      onClick={onExpand}
      title={[group.toolName, group.args, summary].filter(Boolean).join("\n")}
      role="button"
      tabIndex={0}
      aria-expanded={false}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onExpand();
        }
      }}
    >
      <span className="tool-chip-icon" aria-hidden="true">
        {meta.icon}
      </span>
      <span className="tool-chip-main">
        <span className="tool-chip-name">
          {toolRowTitle(group.toolName, meta.label, group.status)}
        </span>
        {subtitle && <span className="tool-chip-detail">{subtitle}</span>}
      </span>
      <span className="tool-chip-trail">
        {summary && summary !== subtitle && (
          <span className="tool-chip-out">{summary}</span>
        )}
        {group.ms != null && <span className="tool-chip-ms">{group.ms}ms</span>}
        <ToolStatusPill status={group.status} />
        <ChevronRight size={12} className="tool-chip-chevron" aria-hidden="true" />
      </span>
    </div>
  );
}

function ToolGroupView({
  group,
  expanded,
  onToggle,
  mcpPrefixes,
  inBatch = false,
}: {
  group: ToolGroup;
  expanded: boolean;
  onToggle: () => void;
  mcpPrefixes: { id: string; prefix: string }[];
  inBatch?: boolean;
}) {
  const meta = toolMeta(group.toolName, mcpPrefixes);
  const subtitle = pickToolRowSubtitle(group);
  const summary = toolCollapsedSummary(group);
  const hasOutput = group.steps.some((s) => s.output);
  const nonTrivialSteps = group.steps.filter(
    (s) => s.kind !== "start" && !(s.kind === "done" && s.output),
  );
  const simpleBody = nonTrivialSteps.length === 0 && hasOutput;

  const rootCls = inBatch
    ? `tool-run-item-inner status-${group.status} ${expanded ? "is-expanded-view" : "is-collapsed"}`
    : `tool-card status-${group.status} ${expanded ? "is-expanded-view" : "is-collapsed"}`;

  return (
    <div className={rootCls}>
      <div
        className="tool-card-header"
        role="button"
        tabIndex={0}
        aria-expanded={expanded}
        onClick={onToggle}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            onToggle();
          }
        }}
      >
        <span className="tool-card-icon" aria-hidden="true">
          {meta.icon}
        </span>
        <div className="tool-card-title-wrap">
          <div className="tool-card-title-row">
            <span className="tool-card-title">
              {toolRowTitle(group.toolName, meta.label, group.status)}
            </span>
            {meta.source && (
              <span className="tool-source-chip" title={`Tool backend: ${meta.source.source}`}>
                {meta.source.source}
              </span>
            )}
          </div>
          {!expanded && subtitle && (
            <span className="tool-card-arg-line">{subtitle}</span>
          )}
        </div>
        <div className="tool-card-trail">
          {group.ms != null && <span className="tool-card-ms">{group.ms}ms</span>}
          {hasOutput && !expanded && summary && summary !== subtitle && (
            <span className="tool-card-out" title={summary}>
              {summary}
            </span>
          )}
          <ToolStatusPill status={group.status} />
          <ChevronRight size={12} className="tool-card-chevron" aria-hidden="true" />
        </div>
      </div>

      {expanded &&
        (simpleBody ? (
          <ToolDetailBody group={group} mcpPrefixes={mcpPrefixes} variant="card" />
        ) : (
          <div className="tool-timeline">
            {group.steps.map((s, i) => (
              <div
                key={i}
                className={`tool-timeline-node kind-${s.kind}${
                  s.kind === "done" ? (s.ok ? " is-ok" : " is-err") : ""
                }`}
              >
                <span className="tool-timeline-dot" />
                <div className="tool-timeline-content">
                  <ToolStepView step={s} toolName={group.toolName} />
                </div>
              </div>
            ))}
          </div>
        ))}
    </div>
  );
}

function ToolStepView({
  step,
  toolName,
  inline = false,
}: {
  step: ToolStep;
  toolName?: string;
  inline?: boolean;
}) {
  const icon =
    step.kind === "done" ? (step.ok ? "✓" : "✗") : STEP_ICON[step.kind] || "·";

  // Interim assistant step — render as a markdown snippet with a tag.
  if (step.kind === "interim") {
    return (
      <div className="tool-step kind-interim">
        <span className="tool-step-icon">{icon}</span>
        <div className="tool-interim-note">
          <span className="tool-interim-tag">Assistant</span>
          <Markdown>{step.text}</Markdown>
        </div>
      </div>
    );
  }

  // Reasoning step inside a tool group — collapsed by default (legacy).
  if (step.kind === "reasoning") {
    const text = step.output || step.text;
    if (!text) return null;
    return (
      <ToolReasoningNote
        text={text}
        original={step.original}
        stepKey={`rs-${step.index}`}
      />
    );
  }

  // Tool output step (done with output).
  if (step.kind === "done" && step.output) {
    return (
      <ToolStepOutput
        output={step.output}
        outputKey={`step-${step.index}`}
        toolName={toolName ?? step.name ?? undefined}
        inline={inline}
      />
    );
  }

  const colorClass =
    step.kind === "done"
      ? step.ok
        ? "kind-done-ok"
        : "kind-done-err"
      : step.kind === "warn"
        ? "kind-warn"
        : step.kind === "approval-pending"
          ? "kind-approval-pending"
          : "";

  return (
    <div className={`tool-step ${colorClass}`}>
      <span className="tool-step-icon">{icon}</span>
      <span className="tool-step-text">{step.text}</span>
    </div>
  );
}

function ToolReasoningNote({
  text,
  original,
  stepKey,
}: {
  text: string;
  original?: string | null;
  stepKey: string;
}) {
  const [expanded, setExpanded] = useState(false);
  const [viewMode, setViewMode] = useState<"summary" | "original">("summary");
  const hasOriginal = reasoningHasDistinctOriginal(text, original);
  const activeText = viewMode === "original" && hasOriginal ? original! : text;
  const normalized = normalizeReasoningText(activeText);
  if (!normalized) return null;
  const lineCount = normalized.split("\n").filter((l) => l.trim()).length;

  return (
    <div
      className={`tool-reasoning-note${expanded ? " is-expanded" : " is-collapsed"}`}
      key={stepKey}
    >
      <div className="tool-reasoning-head">
        <span className="tool-reasoning-label">思考过程</span>
        <span className="tool-reasoning-meta">
          {lineCount} 行 · {normalized.length.toLocaleString()} 字符
        </span>
        <button
          type="button"
          className="tool-reasoning-toggle"
          onClick={(e) => {
            e.stopPropagation();
            setExpanded((v) => !v);
          }}
        >
          {expanded ? "收起" : "展开思考"}
        </button>
      </div>
      {expanded && (
        <>
          {hasOriginal && (
            <div className="reasoning-view-toggle">
              <button
                type="button"
                className={`reasoning-view-btn${viewMode === "summary" ? " is-active" : ""}`}
                onClick={(e) => {
                  e.stopPropagation();
                  setViewMode("summary");
                }}
              >
                摘要
              </button>
              <button
                type="button"
                className={`reasoning-view-btn${viewMode === "original" ? " is-active" : ""}`}
                onClick={(e) => {
                  e.stopPropagation();
                  setViewMode("original");
                }}
              >
                原文
              </button>
            </div>
          )}
          <div className="tool-reasoning-body">
            <div className="reasoning-md prose-chat">
              <Markdown>{normalized}</Markdown>
            </div>
          </div>
        </>
      )}
    </div>
  );
}
