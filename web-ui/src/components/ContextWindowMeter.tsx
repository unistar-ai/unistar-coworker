import { useStore } from "../store/wsStore";
import { formatTokens } from "../tabs/chat/parser";

/** Compact LLM context usage meter (header / chat toolbar). */
export default function ContextWindowMeter({
  className = "",
  showLabel = true,
}: {
  className?: string;
  showLabel?: boolean;
}) {
  const ctx = useStore((s) => s.chat_context);
  if (!ctx) return null;

  const used = (ctx.message_tokens || 0) + (ctx.tools_tokens || 0);
  const budget = ctx.input_budget || 1;
  const limit = ctx.context_limit || budget;
  const pct = Math.min(100, Math.round((used / budget) * 100));
  const level = pct >= 95 ? "err" : pct >= 80 ? "warn" : "";
  const ariaLabel = `上下文 ${formatTokens(used)} / ${formatTokens(budget)}（窗口 ${formatTokens(limit)}）`;

  return (
    <div
      className={`context-window-meter${level ? ` is-${level}` : ""} ${className}`.trim()}
      role="meter"
      aria-valuemin={0}
      aria-valuemax={budget}
      aria-valuenow={used}
      aria-label={ariaLabel}
      title={ariaLabel}
    >
      <span className="context-window-track">
        <span className="context-window-fill" style={{ width: `${pct}%` }} />
      </span>
      {showLabel && (
        <span className="context-window-label">
          {formatTokens(used)}/{formatTokens(budget)}
        </span>
      )}
    </div>
  );
}
