import { useCallback, useEffect, useRef } from "react";
import { useStore } from "./store/wsStore";
import { WsConnection } from "./lib/ws";
import { readTokenFromUrl } from "./lib/auth";
import { apiFetch } from "./lib/api";
import type { WebSnapshot } from "./store/protocol";
import Layout from "./components/Layout";
import StateGate from "./components/StateGate";
import { useToast } from "./components/ToastProvider";

const TAB_LABELS: Record<string, string> = {
  chat: "Chat",
  approvals: "Approvals",
  logs: "Logs",
  config: "Config",
};

export default function App() {
  const applyWsMessage = useStore((s) => s.applyWsMessage);
  const setConnection = useStore((s) => s.setConnection);
  const applySnapshot = useStore((s) => s.applySnapshot);
  const setStatusError = useStore((s) => s.setStatusError);
  const hasSnapshot = useStore((s) => s.hasSnapshot);
  const toast = useToast();
  const wsRef = useRef<WsConnection | null>(null);
  const fetchStateRef = useRef<() => Promise<void>>(async () => {});

  // fetchState is stable via ref so the effect doesn't re-run.
  const fetchState = useCallback(async () => {
    const res = await apiFetch<WebSnapshot>("/api/state");
    if (res.ok && res.data) {
      applySnapshot(res.data);
      setStatusError(null);
    } else {
      // Surface the failure so the StateGate can render a retry card instead
      // of leaving the user stuck on "connecting…".
      setStatusError(res.error || `state fetch failed (${res.status})`);
      // If we already had a snapshot (i.e. this was a reconnect refetch, not
      // the initial load), also show a toast so the user notices.
      if (hasSnapshot) {
        toast.error(`Reconnect failed: ${res.error || res.status}`);
      }
    }
  }, [applySnapshot, setStatusError, toast, hasSnapshot]);

  useEffect(() => {
    fetchStateRef.current = fetchState;
  }, [fetchState]);

  useEffect(() => {
    readTokenFromUrl();

    const onReconnect = () => {
      void fetchStateRef.current();
    };

    const ws = new WsConnection(
      applyWsMessage,
      (connected, attempts) => setConnection(connected, attempts),
      onReconnect, // refetch full state after reconnect to recover missed patches
    );
    wsRef.current = ws;
    ws.connect();

    // Also fetch initial state in parallel (WS sends a snapshot on connect,
    // but this covers the race where the WS snapshot arrives before we're
    // subscribed, and surfaces 401/network errors immediately).
    void fetchState();

    return () => {
      ws.close();
      wsRef.current = null;
    };
  }, [applyWsMessage, applySnapshot, setConnection, fetchState]);

  const retry = useCallback(() => {
    setStatusError(null);
    void fetchStateRef.current();
    wsRef.current?.connect();
  }, [setStatusError]);

  return (
    <StateGate retry={retry}>
      <Layout tabLabels={TAB_LABELS} />
    </StateGate>
  );
}

