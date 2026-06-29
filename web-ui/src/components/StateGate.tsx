import type { ReactNode } from "react";
import { useStore } from "../store/wsStore";
import { AlertTriangle, RefreshCw, ExternalLink, Loader2 } from "lucide-react";

interface StateGateProps {
  retry: () => void;
  children: ReactNode;
}

/**
 * Render gate that covers two pre-snapshot states:
 *  - still loading (no snapshot yet, no error) → minimal spinner splash
 *  - initial load failed (no snapshot yet, error set) → error card with Retry
 *
 * Once a snapshot has been applied (hasSnapshot === true) the gate is
 * transparent and the real Layout renders. Subsequent disconnects are shown
 * inline by the ConnDot/Status in the topbar, not here.
 */
export default function StateGate({ retry, children }: StateGateProps) {
  const hasSnapshot = useStore((s) => s.hasSnapshot);
  const statusError = useStore((s) => s.statusError);
  const connected = useStore((s) => s.connected);

  if (hasSnapshot) return <>{children}</>;

  // No snapshot yet.
  if (!statusError) {
    // Loading: initial fetch in flight, or WS snapshot not yet arrived.
    return (
      <div className="gate gate-loading" role="status" aria-live="polite">
        <Loader2 className="gate-spinner" size={22} aria-hidden="true" />
        <div className="gate-text">Connecting to unistar-coworker…</div>
        {!connected && (
          <div className="gate-sub">establishing websocket session</div>
        )}
      </div>
    );
  }

  // Initial load failed and we never got a snapshot.
  return (
    <div className="gate gate-error" role="alert" aria-live="assertive">
      <AlertTriangle className="gate-icon" size={26} aria-hidden="true" />
      <div className="gate-title">Couldn’t load the workspace</div>
      <div className="gate-detail">{statusError}</div>
      <div className="gate-actions">
        <button type="button" className="btn btn-primary" onClick={retry}>
          <RefreshCw size={14} aria-hidden="true" />
          Retry
        </button>
        <a className="btn btn-ghost" href="/legacy">
          <ExternalLink size={14} aria-hidden="true" />
          Open legacy UI
        </a>
      </div>
    </div>
  );
}
