import type { ReactNode } from "react";
import type { LucideIcon } from "lucide-react";

interface EmptyStateProps {
  icon: LucideIcon;
  title: string;
  description?: string;
  /** Optional primary action (e.g. a Refresh button). */
  action?: ReactNode;
}

/** Centered empty-state with an icon, a one-line title, an optional
 * description, and an optional primary action. Replaces the bare
 * `<div className="empty">text</div>` placeholders across tabs. */
export default function EmptyState({
  icon: Icon,
  title,
  description,
  action,
}: EmptyStateProps) {
  return (
    <div className="empty-state" role="status">
      <Icon className="empty-state-icon" size={28} aria-hidden="true" />
      <div className="empty-state-title">{title}</div>
      {description && <div className="empty-state-desc">{description}</div>}
      {action && <div className="empty-state-action">{action}</div>}
    </div>
  );
}
