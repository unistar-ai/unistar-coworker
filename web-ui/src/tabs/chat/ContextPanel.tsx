import { useEffect, useMemo, useRef, useState } from "react";
import { ChevronRight } from "lucide-react";
import { useStore } from "../../store/wsStore";
import { useChatUiStore } from "../../store/chatUiStore";
import { apiPost } from "../../lib/api";
import Markdown from "../../components/Markdown";
import DetailModal from "../../components/DetailModal";
import { formatTokens, reasoningHasDistinctOriginal, toolMeta } from "./parser";
import {
  ContextToolResultView,
  contextToolPreview,
  parseContextToolTranscript,
} from "./contextToolResult";
import { findContextToolMessageIndex, toolNamesMatch } from "./contextFocus";
import { toolRowTitle } from "./toolDisplay";
import type { ChatContext, SkillBlock } from "../../store/protocol";

function ctxInlineSummary(items: string[], max = 3): string {
  if (!items.length) return "";
  const shown = items.slice(0, max);
  const rest = items.length - shown.length;
  const text = shown.join(", ");
  return rest > 0 ? `${text} +${rest}` : text;
}

/** Render the tools_body as Markdown. If it parses as JSON, pretty-print it
 * inside a ```json fenced block so it gets syntax highlighting; otherwise emit
 * it verbatim (it may already be markdown prose). */
export function toolsBodyAsMarkdown(body: string): string {
  const trimmed = body.trim();
  if (!trimmed) return "_(no tool schema text)_";
  try {
    const parsed = JSON.parse(trimmed);
    return "```json\n" + JSON.stringify(parsed, null, 2) + "\n```";
  } catch {
    return body;
  }
}

export interface ContextPanelProps {
  /** Mobile drawer mode: panel is rendered as a fixed overlay (≤900px). */
  mobileOpen?: boolean;
  /** Called when the user closes the mobile drawer (× / Esc / backdrop). */
  onMobileClose?: () => void;
}

export default function ContextPanel({ mobileOpen, onMobileClose }: ContextPanelProps = {}) {
  const ctx = useStore((s) => s.chat_context);
  if (!ctx) return null;

  const close = () => {
    void apiPost("/api/chat/context", { visible: false });
    onMobileClose?.();
  };

  // Esc closes the mobile drawer.
  useEffect(() => {
    if (!mobileOpen) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        close();
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [mobileOpen]);

  const cls = mobileOpen ? "context-pane mobile-open" : "context-pane";

  return (
    <aside className={cls} role="region" aria-label="LLM context">
      {/* Backdrop for the mobile drawer — click to close. Hidden on desktop
       * where the panel is inline (no overlay). */}
      {mobileOpen && (
        <div
          className="ctx-backdrop"
          onClick={close}
          aria-hidden="true"
        />
      )}
      <div className="context-pane-inner">
      <div className="context-header">
        <span className="context-header-title">LLM Context</span>
        {ctx.runtime_context_revision != null && (
          <span className="ctx-rev-badge">rev {ctx.runtime_context_revision}</span>
        )}
        <button
          type="button"
          className="ctx-close"
          onClick={close}
          title="Close"
          aria-label="Close context panel"
        >
          ×
        </button>
      </div>

      <ContextStats ctx={ctx} />
      <ContextTools ctx={ctx} />
      <ContextSkills blocks={ctx.skill_blocks} />
      <ContextMessages ctx={ctx} />
      </div>
    </aside>
  );
}

function ContextStats({ ctx }: { ctx: ChatContext }) {
  const used = (ctx.message_tokens || 0) + (ctx.tools_tokens || 0);
  const budget = ctx.input_budget || 1;
  const pct = Math.min(100, Math.round((used / budget) * 100));
  const barCls = pct >= 95 ? "err" : pct >= 80 ? "warn" : "";

  const trimNote = (() => {
    const note = (ctx.context_summary_note || "").trim();
    if (note) return note;
    const trimmed = ctx.context_trimmed_turns || 0;
    if (trimmed > 0) {
      return trimmed === 1
        ? "1 earlier turn omitted"
        : `${trimmed} earlier turns omitted`;
    }
    return null;
  })();

  return (
    <div className="context-stats">
      <div className="ctx-stat-grid">
        <span className="ctx-chip">
          <span className="ctx-chip-k">Turn</span>
          <strong>{ctx.turn}</strong>
        </span>
        <span className="ctx-chip">
          <span className="ctx-chip-k">Msg</span>
          <strong>
            {formatTokens(ctx.message_tokens)} · {ctx.message_count}
          </strong>
        </span>
        <span className="ctx-chip">
          <span className="ctx-chip-k">Tools</span>
          <strong>{formatTokens(ctx.tools_tokens)}</strong>
        </span>
        <span className="ctx-chip">
          <span className="ctx-chip-k">Skills</span>
          <strong>{formatTokens(ctx.skills_tokens)}</strong>
        </span>
      </div>

      {trimNote && (
        <div className="ctx-trim-note" title="Context trimming / summarization">
          {trimNote}
        </div>
      )}

      <div className="ctx-budget-row">
        <div className={`token-bar ctx-budget-bar ${barCls}`}>
          <span style={{ width: `${Math.max(pct, 0)}%` }} />
        </div>
        <span className="ctx-budget-label">
          {formatTokens(used)} / {formatTokens(budget)}{" "}
          <span className="ctx-budget-of">({pct}%)</span>
        </span>
      </div>
    </div>
  );
}

function ContextTools({ ctx }: { ctx: ChatContext }) {
  const names = ctx.tool_names || [];
  const body = ctx.tools_body || "";
  const [chipsExpanded, setChipsExpanded] = useState(false);
  const [schemaOpen, setSchemaOpen] = useState(false);

  if (!names.length && !body) return null;

  return (
    <div className={`ctx-tools${chipsExpanded ? " chips-expanded" : ""}`}>
      <div
        className="ctx-compact-head"
        role="button"
        tabIndex={0}
        aria-expanded={chipsExpanded}
        onClick={() => setChipsExpanded((e) => !e)}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            setChipsExpanded((v) => !v);
          }
        }}
      >
        <span className="ctx-section-title">Tools ({names.length})</span>
        <span className="ctx-inline-summary" title={names.join(", ")}>
          {ctxInlineSummary(names, 4)}
        </span>
        <span className={`ctx-compact-chevron${chipsExpanded ? " is-expanded" : ""}`}><ChevronRight size={10} /></span>
        <button
          type="button"
          className="ctx-tools-toggle"
          onClick={(e) => {
            e.stopPropagation();
            setSchemaOpen(true);
          }}
        >
          schema
        </button>
      </div>

      <div className="ctx-tool-chips">
        {names.map((name) => (
          <span key={name} className="ctx-tool-chip">
            {name}
          </span>
        ))}
      </div>

      <DetailModal
        open={schemaOpen}
        onOpenChange={setSchemaOpen}
        title="Tool schema"
        subtitle={`${names.length} tools · ${formatTokens(ctx.tools_tokens)} tokens`}
        size="lg"
      >
        <Markdown>{toolsBodyAsMarkdown(body)}</Markdown>
      </DetailModal>
    </div>
  );
}

function ContextSkills({ blocks }: { blocks: SkillBlock[] }) {
  const [listExpanded, setListExpanded] = useState(false);
  const [openSkillKey, setOpenSkillKey] = useState<string | null>(null);

  if (!blocks.length) return null;

  const names = blocks.map((s) => s.name);
  const openSkill = openSkillKey
    ? blocks.find((s) => (s.name || String(s.tokens)) === openSkillKey) ?? null
    : null;

  return (
    <div className={`ctx-skills${listExpanded ? " list-expanded" : ""}`}>
      <div
        className="ctx-compact-head"
        role="button"
        tabIndex={0}
        aria-expanded={listExpanded}
        onClick={() => setListExpanded((e) => !e)}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            setListExpanded((v) => !v);
          }
        }}
      >
        <span className="ctx-section-title">Skills ({blocks.length})</span>
        <span className="ctx-inline-summary" title={names.join(", ")}>
          {ctxInlineSummary(names, 3)}
        </span>
        <span className={`ctx-compact-chevron${listExpanded ? " is-expanded" : ""}`}><ChevronRight size={10} /></span>
      </div>

      <div className="ctx-skills-list">
        {blocks.map((sk) => {
          const key = sk.name || String(sk.tokens);
          return (
            <button
              key={key}
              type="button"
              className={`ctx-skill-chip${sk.always ? " is-always" : ""}`}
              title={
                sk.always
                  ? `${sk.name} · always-on · ${sk.tokens}t`
                  : `View ${sk.name} skill body`
              }
              onClick={(e) => {
                e.stopPropagation();
                setOpenSkillKey(key);
              }}
            >
              {sk.name} · {sk.tokens}t
              {sk.always && <span className="ctx-skill-always-mark" aria-label="always-on">★</span>}
            </button>
          );
        })}
      </div>

      <DetailModal
        open={openSkillKey !== null}
        onOpenChange={(o) => {
          if (!o) setOpenSkillKey(null);
        }}
        title={openSkill?.name ?? "Skill"}
        subtitle={
          openSkill ? `${formatTokens(openSkill.tokens)} tokens` : undefined
        }
        size="lg"
      >
        {openSkill && <SkillFrontmatter sk={openSkill} />}
        {openSkill?.body ? (
          <Markdown>{openSkill.body}</Markdown>
        ) : (
          <div className="ctx-empty">No skill body captured.</div>
        )}
      </DetailModal>
    </div>
  );
}

/** Renders the skill's frontmatter metadata as a structured header above the
 * body Markdown. This is the "opening block" that was previously invisible
 * because the backend strips the YAML frontmatter from the body before
 * sending it. Shows: description, argument-hint, always-on badge, tools (as
 * individual chips), skills (as individual chips), and intent trigger phrases. */
export function SkillFrontmatter({ sk }: { sk: SkillBlock }) {
  const tools = sk.tools ?? [];
  const skills = sk.skills ?? [];
  const triggers = sk.intent_phrases ?? [];
  const bonusKw = sk.intent_bonus_keywords ?? [];
  const hasMeta =
    sk.description ||
    sk.argument_hint ||
    sk.always ||
    skills.length ||
    tools.length ||
    triggers.length;
  if (!hasMeta) return null;
  return (
    <div className="skill-frontmatter">
      {sk.always && (
        <span className="skill-frontmatter-badge is-always" title="always: true — injected on every turn">
          always-on
        </span>
      )}
      {sk.description && (
        <p className="skill-frontmatter-desc">{sk.description}</p>
      )}
      {sk.argument_hint && (
        <div className="skill-frontmatter-arghint" title="argument-hint — what to pass when invoking this skill">
          <span className="skill-frontmatter-arghint-label">arg</span>
          <code className="skill-frontmatter-arghint-value">{sk.argument_hint}</code>
        </div>
      )}
      {tools.length > 0 && (
        <div className="skill-frontmatter-field">
          <span className="skill-frontmatter-field-label">tools</span>
          <div className="skill-frontmatter-chips">
            {tools.map((t) => (
              <span key={t} className="skill-frontmatter-chip is-tool" title={`Tool: ${t}`}>
                {t}
              </span>
            ))}
          </div>
        </div>
      )}
      {skills.length > 0 && (
        <div className="skill-frontmatter-field">
          <span className="skill-frontmatter-field-label">skills</span>
          <div className="skill-frontmatter-chips">
            {skills.map((s) => (
              <span key={s} className="skill-frontmatter-chip is-skill" title={`Technique skill: ${s}`}>
                {s}
              </span>
            ))}
          </div>
        </div>
      )}
      {triggers.length > 0 && (
        <div className="skill-frontmatter-field">
          <span className="skill-frontmatter-field-label">triggers</span>
          <div className="skill-frontmatter-chips">
            {triggers.map((t) => (
              <span key={t} className="skill-frontmatter-chip is-trigger" title={`Intent phrase: "${t}"`}>
                {t}
              </span>
            ))}
            {bonusKw.map((k) => (
              <span key={k} className="skill-frontmatter-chip is-bonus" title={`Bonus keyword: ${k}`}>
                +{k}
              </span>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

function scrollBlockIntoContextScroller(el: HTMLElement) {
  const scroller =
    (el.closest(".context-messages") as HTMLElement | null) ||
    (el.closest(".context-pane-inner") as HTMLElement | null);
  if (!scroller) {
    el.scrollIntoView({ behavior: "smooth", block: "center" });
    return;
  }
  const elRect = el.getBoundingClientRect();
  const scRect = scroller.getBoundingClientRect();
  const delta = elRect.top - scRect.top - scRect.height / 2 + elRect.height / 2;
  scroller.scrollBy({ top: delta, behavior: "smooth" });
}

function ContextMessages({ ctx }: { ctx: ChatContext }) {
  const messages = ctx.messages || [];
  const focus = useChatUiStore((s) => s.contextFocus);
  const focusSeq = useChatUiStore((s) => s.contextFocusSeq);
  const clearContextFocus = useChatUiStore((s) => s.clearContextFocus);
  const listRef = useRef<HTMLDivElement>(null);
  const blockRefs = useRef<Map<number, HTMLDivElement>>(new Map());

  const focusIndex = useMemo(() => {
    if (!focus) return -1;
    const exact = findContextToolMessageIndex(messages, focus);
    if (exact >= 0) return exact;
    // Soft fallback: last tool row with the same name so the button still
    // opens/scrolls somewhere useful when fingerprints are incomplete.
    for (let i = messages.length - 1; i >= 0; i--) {
      const role = (messages[i].role || "").toLowerCase();
      if (!role.includes("tool")) continue;
      const parsed = parseContextToolTranscript(messages[i].content || "");
      if (parsed.toolName && toolNamesMatch(focus.toolName, parsed.toolName)) {
        return i;
      }
    }
    return -1;
  }, [focus, messages, focusSeq]);

  useEffect(() => {
    if (!focus) return;
    if (focusIndex < 0) {
      // Opened panel but no confident match — don't jump to a wrong row.
      const t = window.setTimeout(() => clearContextFocus(), 0);
      return () => window.clearTimeout(t);
    }
    let cancelled = false;
    const run = () => {
      if (cancelled) return;
      const el = blockRefs.current.get(focusIndex);
      if (!el) return;
      el.classList.add("ctx-block-focus-flash");
      scrollBlockIntoContextScroller(el);
      window.setTimeout(() => {
        el.classList.remove("ctx-block-focus-flash");
        clearContextFocus();
      }, 1400);
    };
    // Wait for panel open + expand layout.
    const t = window.setTimeout(() => {
      requestAnimationFrame(() => requestAnimationFrame(run));
    }, 50);
    return () => {
      cancelled = true;
      window.clearTimeout(t);
    };
  }, [focusIndex, focusSeq, focus, clearContextFocus]);

  return (
    <div className="context-messages" ref={listRef}>
      {messages.length === 0 ? (
        <div className="ctx-empty">No context messages yet</div>
      ) : (
        messages.map((m, i) => (
          <ContextMessageBlock
            key={i}
            message={m}
            forceExpanded={i === focusIndex}
            blockRef={(node) => {
              if (node) blockRefs.current.set(i, node);
              else blockRefs.current.delete(i);
            }}
          />
        ))
      )}
    </div>
  );
}

interface CollapsibleMessage {
  role: string;
  tokens: number;
  content: string;
  reasoning_original?: string;
}

function truncateMiddle(s: string, max: number): string {
  if (s.length <= max) return s;
  const half = Math.floor((max - 1) / 2);
  return s.slice(0, half) + "…" + s.slice(s.length - half);
}

function ctxCollapsePolicy(m: CollapsibleMessage): {
  collapsible: boolean;
  previewChars: number;
} {
  const content = m.content || "";
  const lines = content.split("\n").length;
  const role = (m.role || "").toLowerCase();
  if (role.includes("tool")) {
    return { collapsible: content.length > 80 || lines > 2, previewChars: 140 };
  }
  if (role.includes("system")) {
    return { collapsible: content.length > 320 || lines > 6, previewChars: 160 };
  }
  return { collapsible: content.length > 480 || lines > 10, previewChars: 200 };
}

function roleClass(role: string): string {
  const r = (role || "").toLowerCase();
  if (r.includes("user") || r.includes("you")) return "role-user";
  if (r.includes("assistant")) return "role-assistant";
  if (r.includes("tool")) return "role-tool";
  if (r.includes("system")) return "role-system";
  if (r === "reasoning") return "role-reasoning";
  return "role-other";
}

function ContextReasoningToggle({
  viewMode,
  onChange,
}: {
  viewMode: "summary" | "original";
  onChange: (mode: "summary" | "original") => void;
}) {
  return (
    <div className="reasoning-view-toggle ctx-reasoning-toggle">
      <button
        type="button"
        className={`reasoning-view-btn${viewMode === "summary" ? " is-active" : ""}`}
        onClick={(e) => {
          e.stopPropagation();
          onChange("summary");
        }}
      >
        摘要
      </button>
      <button
        type="button"
        className={`reasoning-view-btn${viewMode === "original" ? " is-active" : ""}`}
        onClick={(e) => {
          e.stopPropagation();
          onChange("original");
        }}
      >
        原文
      </button>
    </div>
  );
}

function ContextMessageBlock({
  message,
  forceExpanded = false,
  blockRef,
}: {
  message: CollapsibleMessage;
  forceExpanded?: boolean;
  blockRef?: (node: HTMLDivElement | null) => void;
}) {
  const mcpServers = useStore((s) => s.mcp_servers);
  const mcpPrefixes = useMemo(
    () => mcpServers.map((s) => ({ id: s.id, prefix: s.prefix || `${s.id}_` })),
    [mcpServers],
  );

  const { collapsible, previewChars } = ctxCollapsePolicy(message);
  const [expanded, setExpanded] = useState(!collapsible || forceExpanded);
  const [viewMode, setViewMode] = useState<"summary" | "original">("summary");
  const cls = roleClass(message.role);
  const isTool = (message.role || "").toLowerCase().includes("tool");
  const isReasoning = (message.role || "").toLowerCase() === "reasoning";
  const toolParsed = isTool ? parseContextToolTranscript(message.content) : null;
  const hasOriginal = reasoningHasDistinctOriginal(message.content, message.reasoning_original);
  const activeContent =
    isReasoning && viewMode === "original" && hasOriginal
      ? message.reasoning_original!
      : message.content;

  useEffect(() => {
    if (forceExpanded) setExpanded(true);
  }, [forceExpanded]);

  const roleLabel = (() => {
    if (!isTool || !toolParsed?.toolName) return message.role;
    const meta = toolMeta(toolParsed.toolName, mcpPrefixes);
    const title = toolRowTitle(
      toolParsed.toolName,
      meta.label,
      toolParsed.ok ? "ok" : "err",
    );
    return title;
  })();

  const contentEl = isTool ? (
    <ContextToolResultView content={activeContent} mcpPrefixes={mcpPrefixes} />
  ) : (
    <Markdown>{activeContent}</Markdown>
  );

  const reasoningToggle =
    isReasoning && hasOriginal ? (
      <ContextReasoningToggle viewMode={viewMode} onChange={setViewMode} />
    ) : null;

  if (!collapsible) {
    return (
      <div
        ref={blockRef}
        className={`ctx-block ${cls}${isTool ? " has-tool-result" : ""}`}
        data-tool-name={toolParsed?.toolName ?? undefined}
      >
        <div className="ctx-block-head">
          <span className={`ctx-role ${cls}`}>{roleLabel}</span>
          <span className="ctx-tokens">{message.tokens} tok</span>
        </div>
        {reasoningToggle}
        <div className={`ctx-content${isTool ? " ctx-content-tool" : ""}`}>
          {contentEl}
        </div>
      </div>
    );
  }

  const preview = isTool
    ? contextToolPreview(activeContent, previewChars, mcpPrefixes)
    : truncateMiddle(activeContent.replace(/\s+/g, " ").trim(), previewChars);

  return (
    <div
      ref={blockRef}
      className={`ctx-block collapsible ${cls}${isTool ? " has-tool-result" : ""} ${expanded ? "" : "collapsed"}`}
      data-tool-name={toolParsed?.toolName ?? undefined}
    >
      <div
        className="ctx-block-head clickable"
        role="button"
        tabIndex={0}
        aria-expanded={expanded}
        onClick={() => setExpanded((e) => !e)}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            setExpanded((v) => !v);
          }
        }}
      >
        <span className={`ctx-role ${cls}`}>{roleLabel}</span>
        <span className="ctx-tokens">{message.tokens} tok</span>
        <button type="button" className={`ctx-block-toggle${expanded ? " is-expanded" : ""}`}>
          <ChevronRight size={10} />
        </button>
      </div>
      {!expanded && <div className="ctx-preview">{preview}</div>}
      {expanded && (
        <>
          {reasoningToggle}
          <div className={`ctx-content${isTool ? " ctx-content-tool" : ""}`}>
            {contentEl}
          </div>
        </>
      )}
    </div>
  );
}
