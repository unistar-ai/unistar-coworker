import type { ReactNode } from "react";
import {
  approvalReasonLines,
  classifyReasonDisplayItems,
  type ParsedApprovalDescription,
} from "./parser";

/** Always show Reason: review body for LLM gate, policy text for harness/MCP. */
export default function ApprovalReason({
  parsed,
}: {
  parsed: ParsedApprovalDescription;
}) {
  const items = classifyReasonDisplayItems(approvalReasonLines(parsed));
  if (!items.length) return null;

  const issues = items.filter((i) => i.kind === "issue");
  const suggestions = items.filter((i) => i.kind === "suggestion");

  return (
    <div className="approval-payload-block approval-reason-block">
      <div className="approval-payload-label">Reason</div>
      <div className="approval-reason-body">
        <ul className="approval-reason-issues">
          {issues.map((item, i) => (
            <li key={`issue-${i}`} className="approval-reason-issue">
              {item.riskType && (
                <span className="approval-reason-tag">{formatRiskType(item.riskType)}</span>
              )}
              <div className="approval-reason-text">{renderInlineCode(item.text)}</div>
            </li>
          ))}
        </ul>
        {suggestions.length > 0 && (
          <div className="approval-reason-suggestions">
            <div className="approval-reason-sublabel">Suggestions</div>
            <ol className="approval-reason-tips">
              {suggestions.map((item, i) => (
                <li key={`tip-${i}`}>{renderInlineCode(item.text)}</li>
              ))}
            </ol>
          </div>
        )}
      </div>
    </div>
  );
}

function formatRiskType(rt: string): string {
  return rt
    .toLowerCase()
    .split("_")
    .map((w) => w.charAt(0).toUpperCase() + w.slice(1))
    .join(" ");
}

/** Render `backtick` spans as <code>; keep the rest as text. */
function renderInlineCode(text: string): ReactNode {
  const parts = text.split(/(`[^`]+`)/g);
  if (parts.length === 1) return text;
  return parts.map((part, i) => {
    if (part.length >= 2 && part.startsWith("`") && part.endsWith("`")) {
      return (
        <code key={i} className="approval-reason-code">
          {part.slice(1, -1)}
        </code>
      );
    }
    return <span key={i}>{part}</span>;
  });
}
