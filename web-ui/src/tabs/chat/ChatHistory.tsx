import { useMemo, useRef, useState, useEffect } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { Copy, Check, RefreshCw, MessageSquare, ChevronRight } from "lucide-react";
import Markdown from "../../components/Markdown";
import ReasoningCard from "../../components/ReasoningCard";
import EmptyState from "../../components/EmptyState";
import { useStore } from "../../store/wsStore";
import { apiPost } from "../../lib/api";
import { toolMeta, parseToolArgsString, formatToolArgValue, normalizeReasoningText, reasoningHasDistinctOriginal } from "./parser";
import { ToolOutputView } from "./toolOutput";
import type {
  ChatBlock,
  ChatMessage,
  ToolStep,
  ToolGroup,
  ToolStepKind,
} from "./parser";

export default function ChatHistory({
  blocks,
  scrollRef,
  stickBottom,
  onStickBottomChange,
  searchQuery = "",
  activeMatchKey = "",
}: {
  blocks: ChatBlock[];
  scrollRef: React.RefObject<HTMLDivElement | null>;
  stickBottom: boolean;
  onStickBottomChange?: (stick: boolean) => void;
  searchQuery?: string;
  activeMatchKey?: string;
}) {
  const chatBusy = useStore((s) => s.chat_busy);
  const chatStreaming = useStore((s) => s.chat_streaming);
  const mcpServers = useStore((s) => s.mcp_servers);
  const prevCountRef = useRef(0);

  const mcpPrefixes = useMemo(
    () => mcpServers.map((s) => ({ id: s.id, prefix: s.prefix || `${s.id}_` })),
    [mcpServers],
  );

  // Track which block keys existed on the previous render so we can flag newly
  // appended blocks with an entrance animation. Only newly-appended (not
  // historically replayed) blocks get the .is-new class.
  const prevKeysRef = useRef<Set<string>>(new Set());
  const newKeys = useMemo(() => {
    const prev = prevKeysRef.current;
    const next = new Set(blocks.map((b) => b.key));
    const added = new Set<string>();
    for (const k of next) {
      if (!prev.has(k)) added.add(k);
    }
    prevKeysRef.current = next;
    return added;
  }, [blocks]);

  // Blocks matching the search query (substring on message body or reasoning text).
  const searchMatches = useMemo(() => {
    if (!searchQuery.trim()) return new Set<string>();
    const q = searchQuery.toLowerCase();
    const matches = new Set<string>();
    for (const b of blocks) {
      const text = b.message?.body || b.reasoningText || "";
      if (text.toLowerCase().includes(q)) matches.add(b.key);
    }
    return matches;
  }, [blocks, searchQuery]);

  const virtualizer = useVirtualizer({
    count: blocks.length,
    getScrollElement: () => scrollRef.current,
    getItemKey: (i) => blocks[i].key,
    estimateSize: () => 120,
    overscan: 8,
  });

  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const handler = () => {
      const gap = el.scrollHeight - el.scrollTop - el.clientHeight;
      onStickBottomChange?.(gap < 80);
    };
    el.addEventListener("scroll", handler, { passive: true });
    return () => el.removeEventListener("scroll", handler);
  }, [scrollRef, onStickBottomChange]);

  const count = blocks.length;
  useEffect(() => {
    if (count > prevCountRef.current && stickBottom) {
      virtualizer.scrollToIndex(count - 1, { align: "end" });
    }
    prevCountRef.current = count;
  }, [count, virtualizer, stickBottom]);

  useEffect(() => {
    if (chatBusy && stickBottom && scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [chatBusy, stickBottom, scrollRef]);

  // Follow streaming text growth when stuck to bottom.
  useEffect(() => {
    if (chatStreaming && stickBottom && scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [chatStreaming, stickBottom, scrollRef]);

  // Scroll to the active search match when it changes.
  useEffect(() => {
    if (!activeMatchKey) return;
    const idx = blocks.findIndex((b) => b.key === activeMatchKey);
    if (idx < 0) return;
    if (blocks.length >= VIRTUAL_THRESHOLD) {
      virtualizer.scrollToIndex(idx, { align: "center", behavior: "smooth" });
    } else {
      const el = document.querySelector(`[data-block-key="${activeMatchKey}"]`);
      el?.scrollIntoView({ behavior: "smooth", block: "center" });
    }
  }, [activeMatchKey]); // eslint-disable-line react-hooks/exhaustive-deps

  const VIRTUAL_THRESHOLD = 80;

  if (blocks.length === 0) {
    return (
      <EmptyState
        icon={MessageSquare}
        title="No messages yet"
        description="Ask about your workspace — code, tests, or docs. For GitHub, the agent will ask which repo if you have not named one yet."
      />
    );
  }

  // Short sessions: render all blocks directly (no virtualizer). This avoids
  // the estimate/measure cycle that causes blank gaps and scroll jumps when
  // markdown blocks have variable height. Virtualization only kicks in for
  // long sessions (>= VIRTUAL_THRESHOLD blocks) where DOM size matters.
  // NOTE: use .chat-block-list (not .messages) so we don't create a second
  // scrolling/padding container — the outer .messages in ChatTab already is
  // the scroll container with padding.
  if (blocks.length < VIRTUAL_THRESHOLD) {
    return (
      <div className="chat-block-list">
        {blocks.map((block) => (
          <div
            key={block.key}
            data-block-key={block.key}
            className={`${blockWrapClass(block)}${newKeys.has(block.key) ? " is-new" : ""}${searchMatches.has(block.key) ? " is-search-match" : ""}${activeMatchKey === block.key ? " is-search-active" : ""}`}
          >
            <BlockRenderer block={block} mcpPrefixes={mcpPrefixes} />
          </div>
        ))}
      </div>
    );
  }

  return (
    <>
      <div
        style={{
          height: `${virtualizer.getTotalSize()}px`,
          width: "100%",
          position: "relative",
        }}
      >
          {virtualizer.getVirtualItems().map((vi) => {
            const block = blocks[vi.index];
            return (
              <div
                key={block.key}
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
                <div data-block-key={block.key} className={`${blockWrapClass(block)}${newKeys.has(block.key) ? " is-new" : ""}${searchMatches.has(block.key) ? " is-search-match" : ""}${activeMatchKey === block.key ? " is-search-active" : ""}`}>
                  <BlockRenderer block={block} mcpPrefixes={mcpPrefixes} />
                </div>
              </div>
            );
          })}
      </div>
    </>
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

function BlockRenderer({
  block,
  mcpPrefixes,
}: {
  block: ChatBlock;
  mcpPrefixes: { id: string; prefix: string }[];
}) {
  if (block.type === "message" && block.message) {
    return <MessageView msg={block.message} isLastAssistant={block.isLastAssistant} />;
  }
  if (block.type === "tool-batch") {
    return <ToolBatchView block={block} mcpPrefixes={mcpPrefixes} />;
  }
  if (block.type === "tool-group" && block.group) {
    return <ToolGroupBlockView group={block.group} mcpPrefixes={mcpPrefixes} />;
  }
  if (block.type === "reasoning") {
    return <ReasoningBlockView text={block.reasoningText || ""} original={block.reasoningOriginal} />;
  }
  return null;
}

function MessageView({ msg, isLastAssistant }: { msg: ChatMessage; isLastAssistant?: boolean }) {
  const [copied, setCopied] = useState(false);
  const chatBusy = useStore((s) => s.chat_busy);
  const assistantId = useStore((s) => s.chat_assistant_ids[String(msg.lineIndex)]);

  // System/meta messages get a centered pill style.
  if (msg.role === "system" || msg.role === "meta") {
    const cls = /cleared|^new session/i.test(msg.body)
      ? "msg-system system-session"
      : /approval|denied|approved/i.test(msg.body)
        ? "msg-system system-approval"
        : "msg-system";
    return <div className={cls}>{msg.body}</div>;
  }

  const roleClass =
    msg.role === "you"
      ? "role-you"
      : msg.role === "assistant"
        ? "role-assistant"
        : msg.role === "error"
          ? "role-error"
          : "role-assistant";
  const label =
    msg.role === "you" ? "You" : msg.role === "assistant" ? "Assistant" : msg.role === "error" ? "Error" : msg.badge;

  const onCopy = async () => {
    try {
      await navigator.clipboard.writeText(msg.body);
      setCopied(true);
      setTimeout(() => setCopied(false), 1200);
    } catch {
      /* clipboard unavailable */
    }
  };

  return (
    <div className={`msg-row ${roleClass}`}>
      <div className="msg-label">{label}</div>
      <div className="msg-bubble">
        {msg.md ? (
          <Markdown>{msg.body}</Markdown>
        ) : (
          <div className="whitespace-pre-wrap">{msg.body}</div>
        )}
        <div className="msg-actions">
          <button
            type="button"
            className="msg-copy"
            onClick={onCopy}
            aria-label="Copy message"
            title="Copy message"
          >
            {copied ? <Check size={14} /> : <Copy size={14} />}
          </button>
          {msg.role === "assistant" && assistantId && !chatBusy && (
            <button
              type="button"
              className="msg-regenerate"
              onClick={() =>
                void apiPost("/api/chat/regenerate", { message_id: assistantId })
              }
              aria-label={
                isLastAssistant
                  ? "Regenerate response"
                  : "Branch from this response"
              }
              title={
                isLastAssistant
                  ? "Regenerate response"
                  : "Branch from this response"
              }
            >
              <RefreshCw size={14} />
            </button>
          )}
        </div>
      </div>
    </div>
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
  ok: "OK",
  err: "Failed",
  pending: "Pending",
  running: "Running",
  warn: "Warning",
};

function ToolStatusPill({ status }: { status: string }) {
  const label = STATUS_PILL[status];
  if (!label) return null;
  return <span className={`tool-status-pill status-${status}`}>{label}</span>;
}

function primaryArgLine(args: string | null | undefined): string | null {
  const pairs = parseToolArgsString(args);
  if (!pairs.length) return null;
  const { key, value } = pairs[0];
  if (!value) return key;
  const v = formatToolArgValue(key, value);
  const text = `${key}=${v}`;
  return text.length > 52 ? `${text.slice(0, 51)}…` : text;
}

/** Unified output preview for a collapsed tool group. */
function outputPreview(group: ToolGroup): string | null {
  const step = group.steps.find((s) => s.output);
  if (!step?.output) return null;
  const lines = step.output.split("\n").length;
  if (lines > 1) return `${lines} lines`;
  if (step.output.length > 80) return `${step.output.length} chars`;
  const oneLine = step.output.replace(/\s+/g, " ").trim();
  if (!oneLine) return null;
  return oneLine.length > 64 ? `${oneLine.slice(0, 63)}…` : oneLine;
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
}: {
  group: ToolGroup;
  mcpPrefixes: { id: string; prefix: string }[];
  inBatch?: boolean;
}) {
  const [expanded, setExpanded] = useState(false);
  const compact =
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
      expanded={expanded}
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
  const outHint = outputPreview(group);
  const argLine = primaryArgLine(group.args);

  return (
    <div
      className={`tool-chip status-${group.status} clickable`}
      onClick={onExpand}
      title={[group.toolName, group.args, outHint].filter(Boolean).join("\n")}
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
        <span className="tool-chip-name">{meta.label}</span>
        {(argLine || meta.source) && (
          <span className="tool-chip-detail">
            {argLine || meta.source?.source}
          </span>
        )}
      </span>
      <span className="tool-chip-trail">
        {outHint && <span className="tool-chip-out">{outHint}</span>}
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
  const hasOutput = group.steps.some((s) => s.output);
  const outputSummary = outputPreview(group);
  const argPairs = parseToolArgsString(group.args);
  const argLine = primaryArgLine(group.args);

  // Inside an expanded batch, render without the outer .tool-card border to
  // avoid a "card-in-card" double border; a left status bar carries the
  // status color instead.
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
            <span className="tool-card-title">{meta.label}</span>
            {meta.label !== group.toolName && (
              <span className="tool-card-fn">{group.toolName}</span>
            )}
            {meta.source && (
              <span className="tool-source-chip" title={`Tool backend: ${meta.source.source}`}>
                {meta.source.source}
              </span>
            )}
          </div>
          {!expanded && argLine && (
            <span className="tool-card-arg-line">{argLine}</span>
          )}
          {expanded && argPairs.length > 0 && (
            <div className="tool-arg-chips">
              {argPairs.map((p, i) => (
                <span key={i} className="tool-arg-chip">
                  <span className="tool-arg-k">{p.key}</span>
                  {p.value && (
                    <span className="tool-arg-v">{formatToolArgValue(p.key, p.value)}</span>
                  )}
                </span>
              ))}
            </div>
          )}
        </div>
        <div className="tool-card-trail">
          {group.ms != null && <span className="tool-card-ms">{group.ms}ms</span>}
          {hasOutput && !expanded && outputSummary && (
            <span className="tool-card-out" title={outputSummary}>
              {outputSummary}
            </span>
          )}
          <ToolStatusPill status={group.status} />
          <ChevronRight size={12} className="tool-card-chevron" aria-hidden="true" />
        </div>
      </div>

      {expanded && (
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
                <ToolStepView step={s} />
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function ToolStepView({ step }: { step: ToolStep }) {
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
      <ToolOutputView output={step.output} outputKey={`step-${step.index}`} />
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
        <span className="tool-reasoning-label">Reasoning</span>
        <span className="tool-reasoning-meta">
          {lineCount} lines · {normalized.length.toLocaleString()} chars
        </span>
        <button
          type="button"
          className="tool-reasoning-toggle"
          onClick={(e) => {
            e.stopPropagation();
            setExpanded((v) => !v);
          }}
        >
          {expanded ? "Collapse" : "Show reasoning"}
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
                Summary
              </button>
              <button
                type="button"
                className={`reasoning-view-btn${viewMode === "original" ? " is-active" : ""}`}
                onClick={(e) => {
                  e.stopPropagation();
                  setViewMode("original");
                }}
              >
                Original
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
