import { useMemo, useState } from "react";
import { Brain } from "lucide-react";
import Markdown from "./Markdown";
import { normalizeReasoningText } from "../tabs/chat/parser";

function reasoningLineCount(text: string): number {
  const n = normalizeReasoningText(text);
  if (!n) return 0;
  return n.split("\n").filter((line) => line.trim()).length;
}

function reasoningCharCount(text: string): number {
  return normalizeReasoningText(text).length;
}

export { reasoningLineCount, reasoningCharCount };

export interface ReasoningCardProps {
  text: string;
  /** Raw (uncompressed) thinking trace, when LLM compression was applied.
   *  When present, a Summary/Original toggle is shown. */
  original?: string;
  /** Live zone — expanded by default, pulse animation, plain-text stream. */
  live?: boolean;
  /** History — collapsed when false. */
  expanded?: boolean;
  onToggle?: () => void;
}

/** Reasoning / thinking card — mirrors legacy buildReasoningCard(). */
export default function ReasoningCard({
  text,
  original,
  live = false,
  expanded,
  onToggle,
}: ReasoningCardProps) {
  const [viewMode, setViewMode] = useState<"summary" | "original">("summary");

  const hasOriginal = Boolean(original && original.trim());
  const activeText = viewMode === "original" && hasOriginal ? original! : text;

  const normalized = useMemo(() => normalizeReasoningText(activeText), [activeText]);
  const hasContent = Boolean(normalized);
  const isExpanded = expanded ?? live;
  const showBody = live || isExpanded;

  const metaText = !normalized
    ? live
      ? "streaming…"
      : ""
    : `${reasoningLineCount(activeText)} lines · ${reasoningCharCount(activeText).toLocaleString()} chars`;

  const cardClass = [
    "activity-reasoning",
    live ? "is-live" : "history-reasoning",
    !live && !isExpanded ? "is-collapsed" : "",
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <div className={cardClass}>
      <div className="activity-reasoning-head">
        <span className="activity-icon"><Brain size={14} aria-hidden="true" /></span>
        <div className="activity-reasoning-title-wrap">
          <span className="activity-title">Reasoning</span>
          <span className="activity-reasoning-meta">{metaText}</span>
        </div>
        {!live && hasContent && (
          <button
            type="button"
            className="activity-toggle"
            onClick={(e) => {
              e.stopPropagation();
              onToggle?.();
            }}
          >
            {isExpanded ? "Collapse" : "Show reasoning"}
          </button>
        )}
      </div>
      {showBody && hasOriginal && !live && (
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
      {showBody && (
        <div className={`activity-reasoning-body${live ? " is-live" : ""}`}>
          {!normalized ? (
            live ? (
              <span className="reasoning-empty">Thinking…</span>
            ) : (
              <span className="reasoning-empty">No reasoning captured.</span>
            )
          ) : live ? (
            <>
              <div className="reasoning-plain is-live">{normalized}</div>
              <span className="reasoning-cursor" aria-hidden="true" />
            </>
          ) : (
            <div className="reasoning-md prose-chat">
              <Markdown>{normalized}</Markdown>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
