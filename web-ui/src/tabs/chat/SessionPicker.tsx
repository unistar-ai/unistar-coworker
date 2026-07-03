import { useEffect, useRef, useState } from "react";
import { ChevronDown, Trash2 } from "lucide-react";
import { useToast } from "../../components/ToastProvider";
import { apiDelete, apiFetch, apiPost } from "../../lib/api";
import { useStore } from "../../store/wsStore";
import Skeleton from "../../components/Skeleton";

interface SessionItem {
  id: string;
  title: string;
  created_at: string;
}

export default function SessionPicker() {
  const sessionId = useStore((s) => s.chat_session_id);
  const chatBusy = useStore((s) => s.chat_busy);
  const toast = useToast();
  const [sessions, setSessions] = useState<SessionItem[] | null>(null);
  const [menuOpen, setMenuOpen] = useState(false);
  const [activeIdx, setActiveIdx] = useState(-1);
  const [pendingDeleteId, setPendingDeleteId] = useState<string | null>(null);
  const [deleting, setDeleting] = useState(false);
  const pickerRef = useRef<HTMLDivElement>(null);

  const refreshSessions = () => {
    void apiFetch<SessionItem[]>("/api/chat/sessions").then((res) => {
      if (res.ok && Array.isArray(res.data)) {
        setSessions(res.data);
      } else {
        setSessions([]);
      }
    });
  };

  // Initial load.
  useEffect(() => {
    refreshSessions();
  }, []);

  // Refresh when session changes (new session created or session switched).
  useEffect(() => {
    refreshSessions();
  }, [sessionId]);

  useEffect(() => {
    if (!menuOpen) return;
    const onDoc = (e: MouseEvent) => {
      if (!pickerRef.current?.contains(e.target as Node)) {
        setMenuOpen(false);
        setPendingDeleteId(null);
      }
    };
    document.addEventListener("click", onDoc);
    return () => document.removeEventListener("click", onDoc);
  }, [menuOpen]);

  useEffect(() => {
    if (!pendingDeleteId || deleting) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        setPendingDeleteId(null);
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [pendingDeleteId, deleting]);

  const current = sessions?.find((s) => s.id === sessionId);
  const label = current
    ? `${current.title} · ${current.created_at}`
    : sessionId
      ? sessionId.slice(0, 8)
      : "New session";

  const pick = (id: string | "new") => {
    if (chatBusy || deleting) return;
    setMenuOpen(false);
    setPendingDeleteId(null);
    if (id === "new") {
      void apiPost("/api/chat/sessions/new");
    } else {
      void apiPost(`/api/chat/sessions/${id}`);
    }
  };

  const toggleDeleteConfirm = (e: React.MouseEvent, id: string) => {
    e.stopPropagation();
    if (chatBusy || deleting) return;
    setPendingDeleteId((cur) => (cur === id ? null : id));
  };

  const confirmDelete = async (id: string) => {
    if (deleting) return;
    setDeleting(true);
    const res = await apiDelete(`/api/chat/sessions/${id}`);
    setDeleting(false);
    if (res.ok) {
      setPendingDeleteId(null);
      refreshSessions();
      toast.success("Session deleted");
      return;
    }
    toast.error(res.error || "Failed to delete session");
  };

  // Listbox options: index 0 is "New session", then the sessions list.
  const optionCount = 1 + (sessions?.length ?? 0);
  const chooseAt = (idx: number) => {
    if (idx === 0) pick("new");
    else if (sessions && idx - 1 < sessions.length) pick(sessions[idx - 1].id);
  };

  const onTriggerKeyDown = (e: React.KeyboardEvent) => {
    if (!menuOpen) {
      if (e.key === "ArrowDown" || e.key === "Enter" || e.key === " ") {
        e.preventDefault();
        setMenuOpen(true);
        setActiveIdx(sessions?.findIndex((s) => s.id === sessionId) ?? -1);
      }
      return;
    }
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setActiveIdx((i) => Math.min(optionCount - 1, i + 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setActiveIdx((i) => Math.max(0, i - 1));
    } else if (e.key === "Enter") {
      e.preventDefault();
      if (activeIdx >= 0) chooseAt(activeIdx);
    } else if (e.key === "Escape") {
      e.preventDefault();
      if (pendingDeleteId) setPendingDeleteId(null);
      else setMenuOpen(false);
    }
  };

  return (
    <div className="session-picker" ref={pickerRef}>
      <button
        type="button"
        className="session-picker-trigger"
        disabled={chatBusy || deleting}
        title={sessionId || undefined}
        aria-haspopup="listbox"
        aria-expanded={menuOpen}
        onClick={(e) => {
          e.stopPropagation();
          setMenuOpen((o) => !o);
          setPendingDeleteId(null);
        }}
        onKeyDown={onTriggerKeyDown}
      >
        <span className="session-picker-label">{label}</span>
        <span className={`session-picker-chevron${menuOpen ? " is-open" : ""}`} aria-hidden="true">
          <ChevronDown size={12} />
        </span>
      </button>
      {menuOpen && (
        <div className="session-menu" role="listbox" aria-label="Chat sessions">
          <button
            type="button"
            className={`session-menu-item session-menu-new${activeIdx === 0 ? " is-active" : ""}`}
            onClick={() => pick("new")}
            disabled={chatBusy || deleting}
            role="option"
            aria-selected={activeIdx === 0}
          >
            + New session
          </button>
          {sessions === null && (
            <div className="session-menu-skeleton">
              <Skeleton rows={2} />
            </div>
          )}
          {sessions?.map((s, i) => {
            const optIdx = i + 1;
            const isConfirming = pendingDeleteId === s.id;
            return (
              <div
                key={s.id}
                className={`session-menu-row${s.id === sessionId ? " is-current" : ""}${activeIdx === optIdx ? " is-kbd-focus" : ""}${isConfirming ? " is-confirming" : ""}`}
              >
                {isConfirming ? (
                  <div
                    className="session-delete-inline"
                    role="dialog"
                    aria-labelledby={`session-delete-${s.id}`}
                    onClick={(e) => e.stopPropagation()}
                  >
                    <p id={`session-delete-${s.id}`} className="session-delete-inline-label">
                      Delete &ldquo;{s.title}&rdquo;?
                    </p>
                    <div className="session-delete-inline-actions">
                      <button
                        type="button"
                        className="session-delete-btn session-delete-btn-cancel"
                        disabled={deleting}
                        onClick={() => setPendingDeleteId(null)}
                      >
                        Cancel
                      </button>
                      <button
                        type="button"
                        className="session-delete-btn session-delete-btn-danger"
                        disabled={deleting}
                        onClick={() => void confirmDelete(s.id)}
                      >
                        {deleting ? "…" : "Delete"}
                      </button>
                    </div>
                  </div>
                ) : (
                  <button
                    type="button"
                    className={`session-menu-item${s.id === sessionId ? " is-active" : ""}`}
                    onClick={() => pick(s.id)}
                    disabled={chatBusy || deleting}
                    role="option"
                    aria-selected={s.id === sessionId}
                  >
                    <span className="session-menu-title">{s.title}</span>
                    <span className="session-menu-meta">{s.created_at}</span>
                  </button>
                )}
                <button
                  type="button"
                  className={`session-menu-delete${isConfirming ? " is-active" : ""}`}
                  title={isConfirming ? "Cancel delete" : "Delete session"}
                  aria-label={
                    isConfirming ? `Cancel delete ${s.title}` : `Delete session ${s.title}`
                  }
                  aria-expanded={isConfirming}
                  disabled={chatBusy || deleting}
                  onClick={(e) => toggleDeleteConfirm(e, s.id)}
                >
                  <Trash2 size={12} aria-hidden="true" />
                </button>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
