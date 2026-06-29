import * as Dialog from "@radix-ui/react-dialog";
import { type ReactNode } from "react";
import { X } from "lucide-react";

interface DetailModalProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  /** Optional subtitle / meta shown next to the title. */
  subtitle?: ReactNode;
  children: ReactNode;
  /** Max width preset. */
  size?: "md" | "lg";
}

/**
 * Generic read-only detail modal (Tools schema / Skill body). Built on Radix
 * Dialog so Esc + backdrop click close it, and focus is trapped while open.
 * The body is a scrollable Markdown container.
 */
export default function DetailModal({
  open,
  onOpenChange,
  title,
  subtitle,
  children,
  size = "lg",
}: DetailModalProps) {
  // Radix handles Esc + backdrop. We also stop propagation inside the box so
  // clicks on the content don't bubble to the overlay's onPointerDownOutside.
  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <Dialog.Portal>
        <Dialog.Content
          className={`detail-modal detail-modal-${size}`}
          aria-describedby={undefined}
        >
          <div className="detail-modal-box" onClick={(e) => e.stopPropagation()}>
            <div className="detail-modal-head">
              <div className="detail-modal-head-text">
                <Dialog.Title asChild>
                  <h3>{title}</h3>
                </Dialog.Title>
                {subtitle && (
                  <Dialog.Description className="detail-modal-subtitle">
                    {subtitle}
                  </Dialog.Description>
                )}
              </div>
              <Dialog.Close asChild>
                <button
                  type="button"
                  className="detail-modal-close"
                  aria-label="Close"
                  title="Close"
                >
                  <X size={16} aria-hidden="true" />
                </button>
              </Dialog.Close>
            </div>
            <div className="detail-modal-body">{children}</div>
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
