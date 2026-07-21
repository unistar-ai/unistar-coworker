import * as Tabs from "@radix-ui/react-tabs";
import { useStore } from "../store/wsStore";
import { apiPost } from "../lib/api";
import Status from "./Status";
import {
  Sun,
  Moon,
  Sparkles,
  MessageSquare,
  Hand,
  ScrollText,
  Settings,
  type LucideIcon,
} from "lucide-react";
import { useTheme } from "next-themes";
import { lazy, Suspense, useEffect, useState } from "react";
import Skeleton from "./Skeleton";
import Footer from "./Footer";
import ContextWindowMeter from "./ContextWindowMeter";

const ApprovalsTab = lazy(() => import("../tabs/approvals/ApprovalsTab"));
const ApprovalModal = lazy(() => import("../tabs/approvals/ApprovalModal"));
const CommandPalette = lazy(() => import("./CommandPalette"));
const ChatTab = lazy(() => import("../tabs/chat/ChatTab"));
const LogsTab = lazy(() => import("../tabs/logs/LogsTab"));
const ConfigTab = lazy(() => import("../tabs/config/ConfigTab"));

interface LayoutProps {
  tabLabels: Record<string, string>;
}

const TAB_ORDER = ["chat", "approvals", "logs", "config"] as const;

const TAB_ICONS: Record<string, LucideIcon> = {
  chat: MessageSquare,
  approvals: Hand,
  logs: ScrollText,
  config: Settings,
};

export default function Layout({ tabLabels }: LayoutProps) {
  const tab = useStore((s) => s.tab);
  const tabs = useStore((s) => s.tabs);
  const approvalCount = useStore((s) => s.approvals.length);
  const setTab = useStore((s) => s.setTab);
  const connected = useStore((s) => s.connected);
  const reconnectAttempts = useStore((s) => s.reconnectAttempts);
  const { theme, setTheme } = useTheme();
  const [mounted, setMounted] = useState(false);
  const [cmdOpen, setCmdOpen] = useState(false);

  useEffect(() => setMounted(true), []);

  const onTabChange = (value: string) => {
    setTab(value);
    void apiPost(`/api/tab/${value}`);
  };

  // Global keyboard shortcuts:
  //  Ctrl/Cmd+1..4 — switch to the Nth tab (in TAB_ORDER).
  //  Ctrl/Cmd+K    — open command palette.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const mod = e.ctrlKey || e.metaKey;
      if (!mod) return;

      if (e.key >= "1" && e.key <= "4") {
        const idx = Number(e.key) - 1;
        const candidate = TAB_ORDER[idx];
        if (candidate && tabs.includes(candidate)) {
          e.preventDefault();
          onTabChange(candidate);
        }
        return;
      }

      if (e.key.toLowerCase() === "k") {
        e.preventDefault();
        setCmdOpen(true);
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tabs, tab]);

  return (
    <div className="flex h-screen flex-col" style={{ background: "var(--bg)", color: "var(--text)" }}>
      <a href="#main" className="skip-link">
        Skip to main content
      </a>
      <header className="topbar">
        <div className="brand">
          <ConnDot />
          <Sparkles className="brand-icon" size={16} aria-hidden="true" />
          <span>unistar-coworker</span>
        </div>
        <Tabs.Root value={tab} onValueChange={onTabChange}>
          <Tabs.List className="tabs" aria-label="main tabs">
            {TAB_ORDER.filter((t) => tabs.includes(t)).map((t) => {
              const Icon = TAB_ICONS[t];
              const isActive = tab === t;
              return (
                <Tabs.Trigger
                  key={t}
                  value={t}
                  className={`tab ${isActive ? "active" : ""}`}
                >
                  {Icon && (
                    <Icon
                      size={14}
                      className="tab-icon"
                      aria-hidden="true"
                    />
                  )}
                  <span className="tab-label">{tabLabels[t] ?? t}</span>
                  {t === "approvals" && approvalCount > 0 && (
                    <span className="tab-badge">{approvalCount}</span>
                  )}
                </Tabs.Trigger>
              );
            })}
          </Tabs.List>
        </Tabs.Root>
        <div className="topbar-meta">
          <CtxUsage />
          {mounted && (
            <button
              type="button"
              className="theme-toggle"
              onClick={() => setTheme(theme === "dark" ? "light" : "dark")}
              aria-label={
                theme === "dark" ? "Switch to light mode" : "Switch to dark mode"
              }
              title={
                theme === "dark" ? "Switch to light mode" : "Switch to dark mode"
              }
            >
              {theme === "dark" ? <Sun size={16} /> : <Moon size={16} />}
            </button>
          )}
          <Status />
        </div>
      </header>
      {!connected && reconnectAttempts > 0 && (
        <div className="reconnect-banner" role="alert">
          <span className="reconnect-spinner" aria-hidden="true" />
          Reconnecting… (attempt {reconnectAttempts})
        </div>
      )}
      <main id="main" className="flex-1 overflow-hidden" tabIndex={0}>
        <div key={tab} className="tab-content-enter h-full">
          <TabContent tab={tab} />
        </div>
      </main>
      <Footer />
      <Suspense fallback={null}>
        <ApprovalModal />
      </Suspense>
      {cmdOpen && (
        <Suspense fallback={null}>
          <CommandPalette open={cmdOpen} onOpenChange={setCmdOpen} />
        </Suspense>
      )}
    </div>
  );
}

function CtxUsage() {
  const tab = useStore((s) => s.tab);
  if (tab !== "chat") return null;
  return <ContextWindowMeter className="topbar-ctx-meter" />;
}

function ConnDot() {
  const connected = useStore((s) => s.connected);
  const attempts = useStore((s) => s.reconnectAttempts);
  const label = connected ? "Connected" : attempts > 0 ? "Reconnecting" : "Offline";
  const dotClass = connected ? "live" : attempts > 0 ? "reconnecting" : "dead";
  return (
    <span
      className={`brand-dot ${dotClass}`}
      title={`Connection: ${label}`}
      aria-label={`Connection: ${label}`}
    />
  );
}

function TabContent({ tab }: { tab: string }) {
  return (
    <Suspense fallback={<TabLoading />}>
      <TabPanel tab={tab} />
    </Suspense>
  );
}

function TabLoading() {
  return (
    <div className="flex h-full items-start p-4">
      <Skeleton rows={10} className="w-full max-w-3xl" />
    </div>
  );
}

function TabPanel({ tab }: { tab: string }) {
  switch (tab) {
    case "chat":
      return <ChatTab />;
    case "approvals":
      return <ApprovalsTab />;
    case "logs":
      return <LogsTab />;
    case "config":
      return <ConfigTab />;
    default:
      return <Placeholder tab={tab} />;
  }
}

function Placeholder({ tab }: { tab: string }) {
  return (
    <div className="flex h-full items-center justify-center text-text-muted">
      <div className="text-center">
        <div className="mb-2 text-lg capitalize">{tab}</div>
        <p className="text-sm">This tab is not implemented yet.</p>
      </div>
    </div>
  );
}
