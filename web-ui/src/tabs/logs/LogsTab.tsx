import { useRef, useState, useEffect, useCallback } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useStore } from "../../store/wsStore";
import { apiPost } from "../../lib/api";
import EmptyState from "../../components/EmptyState";
import { ScrollText, Copy, Check, ArrowDown } from "lucide-react";

const VIRTUAL_THRESHOLD = 200;

export default function LogsTab() {
  const logs = useStore((s) => s.logs);
  const logFilter = useStore((s) => s.log_filter);
  const scrollRef = useRef<HTMLDivElement>(null);
  const [stickBottom, setStickBottom] = useState(true);
  const prevCount = useRef(0);

  const virtualizer = useVirtualizer({
    count: logs.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => 28,
    overscan: 10,
    enabled: logs.length >= VIRTUAL_THRESHOLD,
  });

  const scrollToBottom = useCallback(() => {
    const el = scrollRef.current;
    if (!el) return;
    el.scrollTop = el.scrollHeight;
  }, []);

  // Auto-scroll when new logs arrive and user was at bottom.
  useEffect(() => {
    if (logs.length > prevCount.current && stickBottom) {
      requestAnimationFrame(scrollToBottom);
    }
    prevCount.current = logs.length;
  }, [logs.length, stickBottom, scrollToBottom]);

  const onScroll = useCallback(() => {
    const el = scrollRef.current;
    if (!el) return;
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 30;
    setStickBottom(atBottom);
  }, []);

  const newCount = stickBottom ? 0 : logs.length - prevCount.current;

  return (
    <div className="panel log-list">
      <div className="toolbar">
        <button
          type="button"
          className="btn btn-ghost"
          onClick={() => void apiPost("/api/logs/filter")}
        >
          Filter: {logFilter || "all"}
        </button>
      </div>
      {logs.length === 0 ? (
        <EmptyState
          icon={ScrollText}
          title="No logs"
          description="Runtime logs will stream here as the agent works."
        />
      ) : (
        <div ref={scrollRef} className="log-scroll" onScroll={onScroll}>
          {logs.length < VIRTUAL_THRESHOLD ? (
            <div>
              {logs.map((l, i) => (
                <LogRow key={i} log={l} />
              ))}
            </div>
          ) : (
            <div
              style={{
                height: `${virtualizer.getTotalSize()}px`,
                width: "100%",
                position: "relative",
              }}
            >
              {virtualizer.getVirtualItems().map((vi) => {
                const l = logs[vi.index];
                return (
                  <div
                    key={vi.index}
                    data-index={vi.index}
                    ref={virtualizer.measureElement}
                    style={{
                      position: "absolute",
                      top: 0,
                      left: 0,
                      width: "100%",
                      transform: `translateY(${vi.start}px)`,
                    }}
                  >
                    <LogRow log={l} />
                  </div>
                );
              })}
            </div>
          )}
        </div>
      )}
      {!stickBottom && logs.length > 0 && (
        <button
          type="button"
          className="log-scroll-fab"
          onClick={() => {
            setStickBottom(true);
            scrollToBottom();
          }}
          aria-label="Jump to latest logs"
          title="Jump to latest"
        >
          <ArrowDown size={16} />
          {newCount > 0 && <span className="log-scroll-fab-badge">{newCount}</span>}
        </button>
      )}
    </div>
  );
}

function LogRow({ log }: { log: { level: string; message: string; ts: string } }) {
  const level = (log.level || "info").toLowerCase();
  const [copied, setCopied] = useState(false);
  const onCopy = async (e: React.MouseEvent) => {
    e.stopPropagation();
    try {
      await navigator.clipboard.writeText(`[${log.ts}] ${log.level}: ${log.message}`);
      setCopied(true);
      setTimeout(() => setCopied(false), 1200);
    } catch {
      /* clipboard unavailable */
    }
  };
  return (
    <div className="log-row">
      <span className="log-ts">
        {new Date(log.ts).toLocaleTimeString()}
      </span>
      <div>
        <span className={`log-level log-level-${level}`}>{level}</span>
        <span className="log-msg">{log.message}</span>
      </div>
      <button
        type="button"
        className={`log-copy${copied ? " is-copied" : ""}`}
        onClick={onCopy}
        aria-label="Copy log line"
        title="Copy"
      >
        {copied ? <Check size={13} /> : <Copy size={13} />}
      </button>
    </div>
  );
}
