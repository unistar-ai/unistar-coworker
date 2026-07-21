import { useEffect, useMemo, useRef, useState } from "react";
import { Check, ChevronRight, Lightbulb, Wrench, X } from "lucide-react";
import type { BlockRendererProps } from "./AgentTurnCard";
import type { DisplayStep } from "./partDisplay";
import ToolDetailBody from "./ToolDetailBody";
import { BOTTOM_COLLAPSE_TOOL_THRESHOLD } from "./liveHeader";

export interface TurnProcessPanelProps {
  steps: DisplayStep[];
  summary: string;
  summaryShimmer?: boolean;
  stats: { tools: number; thoughts: number };
  mcpPrefixes: { id: string; prefix: string }[];
  renderBlock: (props: BlockRendererProps) => React.ReactNode;
  defaultCollapsed?: boolean;
  variant?: "history" | "live";
  /** Live turn still running — enables preview strip + shimmer styling. */
  isLiveProgress?: boolean;
  /** Live transport rows — shown in expanded list and collapsed live preview. */
  extraExpanded?: React.ReactNode;
}

const LIVE_PREVIEW_LIMIT = 3;

/** Shared outer process fold — history turns and Live zone. */
export default function TurnProcessPanel({
  steps,
  summary,
  summaryShimmer = false,
  stats,
  mcpPrefixes,
  renderBlock,
  defaultCollapsed = true,
  variant = "history",
  isLiveProgress = false,
  extraExpanded,
}: TurnProcessPanelProps) {
  const [collapsed, setCollapsed] = useState(defaultCollapsed);
  const [previewDismissed, setPreviewDismissed] = useState(false);
  const isLive = variant === "live";
  const previewRef = useRef<HTMLDivElement | null>(null);
  const hasExtra = extraExpanded != null;
  const showLivePreview =
    isLive &&
    isLiveProgress &&
    collapsed &&
    !previewDismissed &&
    (steps.length > 0 || hasExtra);
  const previewSteps = useMemo(
    () => (showLivePreview ? steps.slice(-LIVE_PREVIEW_LIMIT) : []),
    [showLivePreview, steps],
  );
  const historyPeek = useMemo(() => {
    if (isLive || !collapsed || steps.length === 0) return null;
    const tools = steps.filter((s) => s.kind === "tool");
    if (tools.length > 0) {
      return {
        step: tools[tools.length - 1],
        toolCount: tools.length,
      };
    }
    for (let i = steps.length - 1; i >= 0; i--) {
      const s = steps[i];
      if (s.subtitle || s.title) {
        return { step: s, toolCount: 0 };
      }
    }
    return null;
  }, [isLive, collapsed, steps]);
  const showBottomCollapse = !collapsed && stats.tools > BOTTOM_COLLAPSE_TOOL_THRESHOLD;
  const summaryAria = [
    summary,
    stats.tools > 0 ? `${stats.tools} 次工具调用` : null,
    stats.thoughts > 0 ? `${stats.thoughts} 次思考` : null,
  ]
    .filter(Boolean)
    .join(" · ");

  useEffect(() => {
    if (!isLiveProgress) setPreviewDismissed(false);
  }, [isLiveProgress]);

  useEffect(() => {
    if (!showLivePreview) return;
    const el = previewRef.current;
    if (!el) return;
    el.scrollTop = el.scrollHeight;
  }, [showLivePreview, previewSteps, hasExtra]);

  if (steps.length === 0 && !hasExtra) return null;

  const summaryRow = (
    <>
      <span className="chat-process-summary-icons" aria-hidden="true">
        {(stats.tools > 0 || isLive) && <Wrench size={14} strokeWidth={2} />}
        {stats.thoughts > 0 && <Lightbulb size={14} strokeWidth={2} />}
      </span>
      <span
        className={`chat-process-summary-text${summaryShimmer ? " is-shimmer" : ""}`}
      >
        {summary}
      </span>
      <ChevronRight
        size={15}
        className={`chat-process-chevron${collapsed ? "" : " is-open"}`}
        aria-hidden="true"
      />
    </>
  );

  return (
    <section
      className={`chat-process-panel${collapsed ? " is-collapsed" : " is-expanded"}${
        isLive ? " is-live-variant" : ""
      }${historyPeek ? " has-history-peek" : ""}${
        showLivePreview ? " has-live-preview" : ""
      }`}
      aria-busy={isLive && isLiveProgress ? true : undefined}
    >
      {historyPeek ? (
        <button
          type="button"
          className="chat-process-fold-hit"
          aria-expanded={!collapsed}
          aria-label={summaryAria}
          onClick={() => setCollapsed((c) => !c)}
        >
          <span className="chat-process-summary is-embedded">{summaryRow}</span>
          <span className="chat-process-history-peek">
            <span className="chat-process-history-peek-icon">{historyPeek.step.icon}</span>
            <span className="chat-process-history-peek-body">
              {historyPeek.step.kind === "tool" ? (
                <>
                  <span className="chat-process-history-peek-label">{historyPeek.step.title}</span>
                  {historyPeek.step.subtitle && (
                    <>
                      <span className="chat-process-history-peek-sep" aria-hidden="true">
                        ·
                      </span>
                      <span className="chat-process-history-peek-cmd">{historyPeek.step.subtitle}</span>
                    </>
                  )}
                  {historyPeek.toolCount > 1 && (
                    <span className="chat-process-history-peek-count">×{historyPeek.toolCount}</span>
                  )}
                </>
              ) : (
                <>
                  <span className="chat-process-history-peek-label">{historyPeek.step.title}</span>
                  {historyPeek.step.subtitle && (
                    <>
                      <span className="chat-process-history-peek-sep" aria-hidden="true">
                        ·
                      </span>
                      <span className="chat-process-history-peek-cmd is-thought">
                        {historyPeek.step.subtitle}
                      </span>
                    </>
                  )}
                </>
              )}
            </span>
            {historyPeek.step.statusKind === "ok" && (
              <span className="chat-process-history-peek-ok" aria-hidden="true">
                <Check size={13} strokeWidth={2.5} />
              </span>
            )}
            {(historyPeek.step.statusKind === "err" || historyPeek.step.statusKind === "warn") && (
              <span className="chat-process-history-peek-err" aria-hidden="true">
                <X size={13} strokeWidth={2.5} />
              </span>
            )}
            {(historyPeek.step.statusKind === "running" || historyPeek.step.statusKind === "pending") && (
              <span className="chat-process-history-peek-running" aria-hidden="true">
                <span className="tool-spinner" />
              </span>
            )}
          </span>
        </button>
      ) : (
        <button
          type="button"
          className="chat-process-summary"
          aria-expanded={!collapsed}
          aria-label={summaryAria}
          onClick={() => setCollapsed((c) => !c)}
        >
          {summaryRow}
        </button>
      )}

      {!historyPeek && <div className="chat-process-divider" aria-hidden="true" />}

      {showLivePreview && (
        <div className="chat-process-live-preview">
          <button
            type="button"
            className="chat-process-live-preview-dismiss"
            aria-label="关闭预览"
            onClick={(e) => {
              e.stopPropagation();
              setPreviewDismissed(true);
            }}
          >
            <X size={13} strokeWidth={2} />
          </button>
          <div
            ref={previewRef}
            className="chat-process-live-preview-scroll"
            // @ts-expect-error React 19 supports inert; keep for a11y inert subtree
            inert=""
          >
            {previewSteps.map((step) => (
              <div
                key={step.key}
                className={`chat-process-preview-row kind-${step.kind}${
                  step.isLive ? " is-live" : ""
                }`}
              >
                <span className="chat-process-preview-icon" aria-hidden="true">
                  {step.icon}
                </span>
                <span className="chat-process-preview-text">
                  <span className="chat-process-preview-title">{step.title}</span>
                  {step.subtitle && (
                    <span className="chat-process-preview-subtitle">{step.subtitle}</span>
                  )}
                </span>
                {step.status && (
                  <span
                    className={`chat-process-preview-status status-${step.statusKind || "ok"}`}
                  >
                    {step.status}
                  </span>
                )}
              </div>
            ))}
            {extraExpanded}
          </div>
        </div>
      )}

      {!collapsed && (
        <div className={`chat-process-steps${isLive ? " chat-process-steps-live" : ""}`}>
          {steps.map((step) => (
            <ProcessStepRow
              key={step.key}
              step={step}
              mcpPrefixes={mcpPrefixes}
              renderBlock={renderBlock}
              autoExpand={Boolean(step.isLive && step.kind === "tool")}
            />
          ))}
          {extraExpanded}
        </div>
      )}

      {showBottomCollapse && (
        <button
          type="button"
          className="chat-process-collapse-bottom"
          onClick={() => setCollapsed(true)}
        >
          <span className="chat-process-collapse-line" aria-hidden="true" />
          <span>收起</span>
          <span className="chat-process-collapse-line" aria-hidden="true" />
        </button>
      )}
    </section>
  );
}

function commentBodyLength(step: DisplayStep): number {
  if (step.kind !== "comment") return 0;
  const block = step.detailBlock;
  if (block.type === "message" && block.message) {
    return block.message.body.length;
  }
  return step.subtitle?.length ?? 0;
}

function ProcessStepRow({
  step,
  mcpPrefixes,
  renderBlock,
  autoExpand = false,
}: {
  step: DisplayStep;
  mcpPrefixes: { id: string; prefix: string }[];
  renderBlock: (props: BlockRendererProps) => React.ReactNode;
  autoExpand?: boolean;
}) {
  const isComment = step.kind === "comment";
  const commentLong = isComment && commentBodyLength(step) > 140;
  const canExpand =
    step.kind === "thought" || step.kind === "tool" || (isComment && commentLong);
  const [detailOpen, setDetailOpen] = useState(autoExpand && step.kind === "tool");

  const liveCls = step.isLive ? " is-live" : "";
  const isRunning = step.statusKind === "running" || step.statusKind === "pending";
  const showStatus =
    step.kind === "tool" &&
    (Boolean(step.status) ||
      step.statusKind === "ok" ||
      step.statusKind === "err" ||
      isRunning);
  const thoughtInline = step.kind === "thought" && !detailOpen && Boolean(step.subtitle);
  const commentInline = isComment && !detailOpen;
  // Collapsed tool rows keep title+subtitle inline; expanded detail owns args/result.
  const toolInline = step.kind === "tool" && !detailOpen && Boolean(step.subtitle);
  const toolGroup =
    step.kind === "tool" &&
    step.detailBlock.type === "tool-group" &&
    step.detailBlock.group
      ? step.detailBlock.group
      : null;

  if (isComment && !commentLong) {
    return (
      <div className={`chat-process-row kind-comment is-inline-only${liveCls}`}>
        <div className="chat-process-row-main is-static" role="note">
          <span className="chat-process-row-icon" aria-hidden="true">
            {step.icon}
          </span>
          <span className="chat-process-row-comment">{step.subtitle}</span>
        </div>
      </div>
    );
  }

  const main = (
    <>
      <span className="chat-process-row-icon" aria-hidden="true">
        {step.icon}
      </span>
      <span
        className={`chat-process-row-text${
          thoughtInline || commentInline || toolInline ? " is-inline" : ""
        }${commentInline ? " is-comment" : ""}${toolInline ? " is-tool" : ""}`}
      >
        <span className="chat-process-row-title">{step.title}</span>
        {!(step.kind === "tool" && detailOpen) && step.subtitle && (
          <span
            className={`chat-process-row-subtitle${step.subtitleMono ? " is-mono" : ""}`}
          >
            {step.subtitle}
          </span>
        )}
      </span>
      {showStatus && (
        <span className={`chat-process-row-status status-${step.statusKind || "ok"}`}>
          {isRunning ? (
            <span className="tool-spinner" aria-hidden="true" />
          ) : step.statusKind === "err" ? (
            <X size={13} strokeWidth={2.5} aria-hidden="true" />
          ) : step.statusKind === "ok" || !step.statusKind ? (
            <Check size={13} strokeWidth={2.5} aria-hidden="true" />
          ) : null}
          {!(step.kind === "tool" && detailOpen && isRunning) && step.status}
        </span>
      )}
      {canExpand && (
        <span
          className={`chat-process-row-open${detailOpen ? " is-open" : ""}`}
          title={detailOpen ? "收起详情" : "展开详情"}
          aria-hidden="true"
        >
          <ChevronRight size={14} strokeWidth={2} />
        </span>
      )}
    </>
  );

  return (
    <div className={`chat-process-row kind-${step.kind}${detailOpen ? " is-open" : ""}${liveCls}`}>
      {canExpand ? (
        <button
          type="button"
          className="chat-process-row-main"
          aria-expanded={detailOpen}
          onClick={() => setDetailOpen((o) => !o)}
        >
          {main}
        </button>
      ) : (
        <div className="chat-process-row-main" role="group">
          {main}
        </div>
      )}
      {detailOpen && canExpand && (
        <div
          className={`chat-process-row-detail${
            step.kind === "tool" ? " is-tool" : ""
          }`}
        >
          {step.liveDetailBody ? (
            <div className="chat-turn-plain reasoning-live">{step.liveDetailBody}</div>
          ) : toolGroup ? (
            <ToolDetailBody
              group={toolGroup}
              mcpPrefixes={mcpPrefixes}
              variant="process"
            />
          ) : (
            renderBlock({
              block: step.detailBlock,
              mcpPrefixes,
              compact: true,
              hideLabel: true,
              inline: true,
            })
          )}
        </div>
      )}
    </div>
  );
}
