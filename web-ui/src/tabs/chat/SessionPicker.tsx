import { useEffect, useRef, useState } from "react";
import { apiFetch, apiPost } from "../../lib/api";
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
  const [sessions, setSessions] = useState<SessionItem[] | null>(null);
  const [menuOpen, setMenuOpen] = useState(false);
  const [activeIdx, setActiveIdx] = useState(-1);
  const pickerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    void apiFetch<SessionItem[]>("/api/chat/sessions").then((res) => {
      if (res.ok && Array.isArray(res.data)) {
        setSessions(res.data);
      } else {
        setSessions([]);
      }
    });
  }, []);

  useEffect(() => {
    if (!menuOpen) return;
    const onDoc = (e: MouseEvent) => {
      if (!pickerRef.current?.contains(e.target as Node)) {
        setMenuOpen(false);
      }
    };
    document.addEventListener("click", onDoc);
    return () => document.removeEventListener("click", onDoc);
  }, [menuOpen]);

  const current = sessions?.find((s) => s.id === sessionId);
  const label = current
    ? `${current.title} · ${current.created_at}`
    : sessionId
      ? sessionId.slice(0, 8)
      : "New session";

  const pick = (id: string | "new") => {
    if (chatBusy) return;
    setMenuOpen(false);
    if (id === "new") {
      void apiPost("/api/chat/sessions/new");
    } else {
      void apiPost(`/api/chat/sessions/${id}`);
    }
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
      setMenuOpen(false);
    }
  };

  return (
    <div className="session-picker" ref={pickerRef}>
      <button
        type="button"
        className="session-picker-trigger"
        disabled={chatBusy}
        title={sessionId || undefined}
        aria-haspopup="listbox"
        aria-expanded={menuOpen}
        onClick={(e) => {
          e.stopPropagation();
          setMenuOpen((o) => !o);
        }}
        onKeyDown={onTriggerKeyDown}
      >
        <span className="session-picker-label">{label}</span>
        <span className="session-picker-chevron" aria-hidden="true">
          {menuOpen ? "▴" : "▾"}
        </span>
      </button>
      {menuOpen && (
        <div className="session-menu" role="listbox" aria-label="Chat sessions">
          <button
            type="button"
            className={`session-menu-item session-menu-new${activeIdx === 0 ? " is-active" : ""}`}
            onClick={() => pick("new")}
            disabled={chatBusy}
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
            return (
              <button
                key={s.id}
                type="button"
                className={`session-menu-item${s.id === sessionId ? " is-active" : ""}${activeIdx === optIdx ? " is-kbd-focus" : ""}`}
                onClick={() => pick(s.id)}
                disabled={chatBusy}
                role="option"
                aria-selected={s.id === sessionId}
              >
                <span className="session-menu-title">{s.title}</span>
                <span className="session-menu-meta">{s.created_at}</span>
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}
