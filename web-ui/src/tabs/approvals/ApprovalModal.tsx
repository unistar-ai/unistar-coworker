import * as Dialog from "@radix-ui/react-dialog";
import { useEffect, useMemo, useRef, useState } from "react";
import { AlertTriangle, Check, Ban } from "lucide-react";
import type { ApprovalDialog } from "../../store/protocol";
import { useStore } from "../../store/wsStore";
import { apiPost } from "../../lib/api";
import {
  parseApprovalDescription,
  buildApprovalPayloadBlocks,
} from "./parser";

export default function ApprovalModal() {
  const dialog = useStore((s) => s.approval_dialog);
  if (!dialog) return null;
  return <ModalInner dialog={dialog} />;
}

function ModalInner({ dialog }: { dialog: ApprovalDialog }) {
  const parsed = useMemo(
    () => parseApprovalDescription(dialog.description, dialog.tool_name),
    [dialog.description, dialog.tool_name],
  );
  const payloadBlocks = useMemo(
    () => buildApprovalPayloadBlocks(dialog.tool_name, dialog.tool_args_json),
    [dialog.tool_name, dialog.tool_args_json],
  );
  const rejectRecommended = parsed.verdict === "REJECT";
  const [armMsLeft, setArmMsLeft] = useState<number>(
    Number(dialog.approve_arm_ms_remaining) || 0,
  );
  const [armed, setArmed] = useState<boolean>(dialog.approve_armed);
  const lastIdRef = useRef<string | null>(null);

  useEffect(() => {
    if (lastIdRef.current !== dialog.id) {
      lastIdRef.current = dialog.id;
      setArmMsLeft(Number(dialog.approve_arm_ms_remaining) || 0);
      setArmed(dialog.approve_armed);
    }
  }, [dialog.id, dialog.approve_armed, dialog.approve_arm_ms_remaining]);

  useEffect(() => {
    if (armed || armMsLeft <= 0) return;
    const start = Date.now();
    const initial = armMsLeft;
    const tick = setInterval(() => {
      const elapsed = Date.now() - start;
      const left = Math.max(0, initial - elapsed);
      setArmMsLeft(left);
      if (left <= 0) {
        setArmed(true);
        clearInterval(tick);
      }
    }, 100);
    return () => clearInterval(tick);
  }, [armed, armMsLeft]);

  const onApprove = () => {
    if (!armed || dialog.deciding) return;
    void apiPost(`/api/approvals/${dialog.id}`, { approve: true });
  };
  const onDeny = () => {
    if (dialog.deciding) return;
    void apiPost(`/api/approvals/${dialog.id}`, { approve: false });
  };

  const okLabel = armed
    ? rejectRecommended
      ? "Approve anyway"
      : "Approve"
    : `Approve (${Math.max(1, Math.ceil(armMsLeft / 50) * 50)}ms)`;

  return (
    <Dialog.Root open>
      <Dialog.Portal>
        <Dialog.Content
          className="approval-modal"
          onPointerDownOutside={(e) => {
            if (!dialog.deciding) {
              e.preventDefault();
              onDeny();
            }
          }}
          onEscapeKeyDown={(e) => {
            if (!dialog.deciding) {
              e.preventDefault();
              onDeny();
            }
          }}
          aria-describedby={undefined}
        >
          <div
            className={`approval-box${rejectRecommended ? " verdict-reject" : ""}`}
            onClick={(e) => e.stopPropagation()}
          >
            <div className="approval-head">
              <div className="approval-head-icon" aria-hidden>
                <AlertTriangle size={20} />
              </div>
              <div className="approval-head-text">
                <Dialog.Title asChild>
                  <h3>{dialog.deciding ? "Processing…" : "Approval required"}</h3>
                </Dialog.Title>
                <div className="approval-subtitle">
                  Mutating tool needs your confirmation
                </div>
                <Dialog.Description className="sr-only">
                  Mutating tool {parsed.toolName} needs your confirmation.
                </Dialog.Description>
              </div>
            </div>

            <div className="approval-tool-row">
              <span className="approval-tool-label">Tool</span>
              <code className="approval-tool-name">{parsed.toolName}</code>
            </div>

            {payloadBlocks.length > 0 && (
              <div className="approval-payload">
                {payloadBlocks.map((b) => (
                  <div key={b.label} className="approval-payload-block">
                    <div className="approval-payload-label">{b.label}</div>
                    <pre className="approval-payload-pre">{b.text}</pre>
                  </div>
                ))}
              </div>
            )}

            <div className="approval-detail">
              {parsed.source === "llm-review" && (
                <div
                  className={`approval-verdict-banner verdict-${(parsed.verdict || "unknown").toLowerCase()}`}
                >
                  <span className="approval-verdict-icon">
                    {parsed.verdict === "REJECT"
                      ? <Ban size={16} />
                      : parsed.verdict === "APPROVE"
                        ? <Check size={16} />
                        : <AlertTriangle size={16} />}
                  </span>
                  <div className="approval-verdict-text">
                    <strong>LLM safety review · {parsed.verdict || "REVIEW"}</strong>
                    <span>
                      {parsed.verdict === "REJECT"
                        ? "Automated review flagged risks — read before approving."
                        : "Review passed — confirm to proceed."}
                    </span>
                  </div>
                </div>
              )}
              {parsed.issues.length > 1 ? (
                <ul className="approval-issues">
                  {parsed.issues.map((issue, i) => (
                    <li key={i}>{issue}</li>
                  ))}
                </ul>
              ) : (
                parsed.summary && (
                  <div className="approval-summary">{parsed.summary}</div>
                )
              )}
            </div>

            {dialog.deciding ? (
              <div className="approval-wait">Sending decision…</div>
            ) : (
              <div className="approval-actions">
                <div className="approval-hint">
                  {rejectRecommended
                    ? "Deny is recommended when safety review rejected the action."
                    : "Mutating action — approve only if you trust this operation."}
                </div>
                <div className="approval-btn-row">
                  <button
                    type="button"
                    className={`btn ${rejectRecommended ? "btn-primary" : "btn-danger"}`}
                    onClick={onDeny}
                  >
                    {rejectRecommended ? "Deny (recommended)" : "Deny"}
                  </button>
                  <button
                    type="button"
                    className={`btn ${rejectRecommended ? "btn-warn" : "btn-primary"}`}
                    disabled={!armed}
                    onClick={onApprove}
                    aria-live="polite"
                  >
                    {okLabel}
                  </button>
                </div>
              </div>
            )}
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
