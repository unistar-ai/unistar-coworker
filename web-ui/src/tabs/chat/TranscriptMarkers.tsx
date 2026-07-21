import { Info } from "lucide-react";

export function NewMessagesSeparator() {
  return (
    <div className="chat-new-messages-separator" role="separator">
      <span className="chat-new-messages-line" aria-hidden="true" />
      <span className="chat-new-messages-label">新消息</span>
      <span className="chat-new-messages-line" aria-hidden="true" />
    </div>
  );
}

export function HistoryCoverageGap({ variant }: { variant: "store" | "memory" }) {
  const label =
    variant === "memory"
      ? "较早的 transcript 行已从内存窗口移除"
      : "会话中还有更早的消息未加载";
  return (
    <div className="chat-history-coverage-gap" role="separator">
      <span className="chat-history-coverage-line" aria-hidden="true" />
      <span className="chat-history-coverage-label">
        <Info size={14} aria-hidden="true" />
        {label}
      </span>
      <span className="chat-history-coverage-line" aria-hidden="true" />
    </div>
  );
}
