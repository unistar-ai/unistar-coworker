import { useStore } from "../store/wsStore";

export default function Status() {
  const status = useStore((s) => s.status);
  const engineBusy = useStore((s) => s.engine_busy);
  const engineTaskLabel = useStore((s) => s.engine_task_label);
  const chatBusy = useStore((s) => s.chat_busy);
  const chatTurnPhase = useStore((s) => s.chat_turn_phase);
  const statusError = useStore((s) => s.statusError);
  const autoApprove = useStore((s) => s.auto_approve_mutations);

  const parts: string[] = [status || "ready"];
  if (engineBusy) parts.push(engineTaskLabel || "task");
  if (chatBusy) parts.push(chatTurnPhase || "chat");
  if (statusError) parts.push(statusError);
  if (autoApprove) parts.push("auto-approve ON");
  const text = parts.join(" · ");

  const cls = statusError
    ? "status is-error"
    : autoApprove
      ? "status is-warn"
      : "status";

  return (
    <div className={cls} title={text} aria-live="polite" aria-atomic="true">
      {text}
    </div>
  );
}
