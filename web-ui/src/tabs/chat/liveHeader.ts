import { useEffect, useRef, useState } from "react";
import { formatTurnProcessSummary, toolMeta } from "./parser";
import { partsProcessStats, type ChatMessagePart } from "./messageParts";
import type { LiveTransportState } from "./liveParts";

/** Cherry `LIVE_HEADER_MIN_DURATION_MS` — avoid flickering tool names. */
export const LIVE_HEADER_MIN_DURATION_MS = 700;

export const BOTTOM_COLLAPSE_TOOL_THRESHOLD = 10;

export type LiveHeaderCandidate = {
  key: string;
  text: string;
  shimmer: boolean;
};

/** Resolve what the process fold header should show. */
export function resolveLiveHeaderCandidate(
  parts: ChatMessagePart[],
  live: LiveTransportState,
  elapsedMs: number | null | undefined,
  mcpPrefixes: { id: string; prefix: string }[] = [],
  preferSummary = false,
): LiveHeaderCandidate {
  const stats = partsProcessStats(parts);
  const settled = formatTurnProcessSummary(stats, elapsedMs);

  if (preferSummary && settled) {
    return { key: `summary:${settled}`, text: settled, shimmer: false };
  }

  const toolName = live.toolRunning || live.toolPending;
  if (toolName) {
    const label = toolMeta(toolName, mcpPrefixes).label;
    const text = live.toolPending ? `等待中 · ${label}` : label;
    return {
      key: `tool:${toolName}:${live.toolPending ? "pending" : "running"}`,
      text,
      shimmer: true,
    };
  }

  if (live.reasoning && !live.streaming) {
    return { key: "thinking", text: "深度思考中", shimmer: true };
  }
  if (live.compressing) {
    return { key: "compressing", text: "压缩上下文中", shimmer: true };
  }
  if (live.activityFlow) {
    const kind = live.activityFlow.kind === "Skill" ? "技能" : "活动";
    return {
      key: `activity:${live.activityFlow.kind}:${live.activityFlow.text}`,
      text: `${kind} · ${live.activityFlow.text}`,
      shimmer: true,
    };
  }

  if (settled) {
    return { key: `summary:${settled}`, text: settled, shimmer: false };
  }

  const elapsed =
    elapsedMs != null && elapsedMs > 0
      ? ` · ${(elapsedMs / 1000).toFixed(elapsedMs < 10000 ? 1 : 0)}s`
      : "";
  return { key: "processing", text: `处理中${elapsed}`, shimmer: true };
}

/** Stabilize live header switches for at least `minDurationMs`. */
export function useStableLiveHeader(
  next: LiveHeaderCandidate,
  isLiveProgress: boolean,
  minDurationMs = LIVE_HEADER_MIN_DURATION_MS,
): LiveHeaderCandidate {
  const [display, setDisplay] = useState(next);
  const displayRef = useRef(next);
  const lastChangeAtRef = useRef(Date.now());
  const pendingRef = useRef<LiveHeaderCandidate | null>(null);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    const clear = () => {
      if (timerRef.current) {
        clearTimeout(timerRef.current);
        timerRef.current = null;
      }
    };

    const commit = (c: LiveHeaderCandidate) => {
      displayRef.current = c;
      lastChangeAtRef.current = Date.now();
      setDisplay(c);
    };

    if (displayRef.current.key === next.key) {
      pendingRef.current = null;
      displayRef.current = next;
      setDisplay(next);
      return clear;
    }

    if (!isLiveProgress) {
      clear();
      pendingRef.current = null;
      commit(next);
      return clear;
    }

    pendingRef.current = next;
    const remaining = Math.max(0, minDurationMs - (Date.now() - lastChangeAtRef.current));
    clear();
    timerRef.current = setTimeout(() => {
      const pending = pendingRef.current;
      if (!pending) return;
      pendingRef.current = null;
      timerRef.current = null;
      commit(pending);
    }, remaining);

    return clear;
  }, [isLiveProgress, minDurationMs, next, next.key, next.text, next.shimmer]);

  if (!isLiveProgress) return next;
  if (displayRef.current.key === next.key) return next;
  return display;
}
