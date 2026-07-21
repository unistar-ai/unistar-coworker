import type { ReactNode } from "react";
import { Bot, User, AlertTriangle } from "lucide-react";
import { formatMessageTime } from "./formatMessageTime";

export interface MessageTurnFrameProps {
  role: "user" | "agent" | "error";
  name: string;
  badge?: string;
  /** ISO-8601 wall-clock time for the message (formatted in the header). */
  timeIso?: string;
  children: ReactNode;
  footer?: ReactNode;
  /** Keep footer visible (e.g. latest assistant). */
  footerPinned?: boolean;
}

/** Cherry-style message frame: avatar + name column + body + optional footer. */
export default function MessageTurnFrame({
  role,
  name,
  badge,
  timeIso,
  children,
  footer,
  footerPinned = false,
}: MessageTurnFrameProps) {
  const timeLabel = formatMessageTime(timeIso);
  return (
    <div className={`message-turn-frame role-${role}${footerPinned ? " footer-pinned" : ""}`}>
      <div className="message-turn-head">
        <span className="message-turn-avatar" aria-hidden="true">
          {role === "user" ? (
            <User size={15} strokeWidth={2} />
          ) : role === "error" ? (
            <AlertTriangle size={15} strokeWidth={2} />
          ) : (
            <Bot size={16} strokeWidth={2} />
          )}
        </span>
        <div className="message-turn-column">
          <div className="message-turn-meta">
            <span className="message-turn-name">{name}</span>
            {badge && <span className="chat-turn-badge">{badge}</span>}
            {timeLabel && timeIso && (
              <time className="message-turn-time" dateTime={timeIso}>
                {timeLabel}
              </time>
            )}
          </div>
          <div className="message-turn-body">{children}</div>
          {footer && <div className="message-turn-footer">{footer}</div>}
        </div>
      </div>
    </div>
  );
}

/** User message in bubble style (Cherry UserBubbleMessage). */
export function UserBubbleFrame({
  children,
  timeIso,
}: {
  children: ReactNode;
  timeIso?: string;
}) {
  const timeLabel = formatMessageTime(timeIso);
  return (
    <div className="message-turn-frame role-user is-bubble">
      <div className="message-user-bubble-col">
        <div className="message-user-bubble-row">
          <div className="message-user-bubble">{children}</div>
          <span className="message-turn-avatar message-user-bubble-avatar" aria-hidden="true">
            <User size={15} strokeWidth={2} />
          </span>
        </div>
        {timeLabel && timeIso && (
          <time className="message-user-bubble-time" dateTime={timeIso}>
            {timeLabel}
          </time>
        )}
      </div>
    </div>
  );
}
