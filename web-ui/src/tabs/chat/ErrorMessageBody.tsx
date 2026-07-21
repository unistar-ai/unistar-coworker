import { useState } from "react";
import { formatChatError } from "./formatChatError";

/** Calm error card for chat `error>` lines / failed turn answers. */
export default function ErrorMessageBody({
  body,
  framed = false,
}: {
  body: string;
  framed?: boolean;
}) {
  const formatted = formatChatError(body);
  const [showRaw, setShowRaw] = useState(false);
  const showToggle = formatted.raw.length > 0 && formatted.raw !== formatted.message;

  const card = (
    <div className="chat-error-card" role="alert">
      <div className="chat-error-card-title">{formatted.title}</div>
      <div className="chat-error-card-message">{formatted.message}</div>
      {(formatted.status || formatted.code) && (
        <div className="chat-error-card-meta">
          {formatted.status && (
            <span className="chat-error-chip">HTTP {formatted.status}</span>
          )}
          {formatted.code && <span className="chat-error-chip">{formatted.code}</span>}
        </div>
      )}
      {showToggle && (
        <button
          type="button"
          className="chat-error-card-toggle"
          onClick={() => setShowRaw((v) => !v)}
        >
          {showRaw ? "收起详情" : "查看原始错误"}
        </button>
      )}
      {showRaw && <pre className="chat-error-card-raw">{formatted.raw}</pre>}
    </div>
  );

  if (framed) {
    return <div className="message-turn-body-inner">{card}</div>;
  }
  return card;
}
