import * as Tabs from "@radix-ui/react-tabs";
import { useStore } from "../store/wsStore";
import { apiPost } from "../lib/api";
import Status from "./Status";
import {
  Sun,
  Moon,
  Sparkles,
  MessageSquare,
  LayoutDashboard,
  GitPullRequest,
  Hand,
  ScrollText,
  Settings,
  type LucideIcon,
} from "lucide-react";
import { useTheme } from "next-themes";
import { useEffect, useState } from "react";
import ApprovalsTab from "../tabs/approvals/ApprovalsTab";
import ApprovalModal from "../tabs/approvals/ApprovalModal";
import CommandPalette from "./CommandPalette";
import ChatTab from "../tabs/chat/ChatTab";
import DashboardTab from "../tabs/dashboard/DashboardTab";
import PrsTab from "../tabs/prs/PrsTab";
import LogsTab from "../tabs/logs/LogsTab";
import ConfigTab from "../tabs/config/ConfigTab";
import Footer from "./Footer";
import { formatTokens } from "../tabs/chat/parser";

interface LayoutProps {
  tabLabels: Record<string, string>;
}

const TAB_ORDER = ["chat", "dashboard", "prs", "approvals", "logs", "config"] as const;

const TAB_ICONS: Record<string, LucideIcon> = {
  chat: MessageSquare,
  dashboard: LayoutDashboard,
  prs: GitPullRequest,
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
  //  Ctrl/Cmd+1..6 — switch to the Nth tab (in TAB_ORDER).
  //  Ctrl/Cmd+K    — focus the chat input (switching to Chat first if needed).
  // Skipped when the user is typing in an input/textarea/contentEditable so we
  // don't swallow printable combos, except for the explicit Cmd/Ctrl modifiers
  // which are safe.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const mod = e.ctrlKey || e.metaKey;
      if (!mod) return;

      // Cmd/Ctrl + 1..6 → tab switch
      if (e.key >= "1" && e.key <= "6") {
        const idx = Number(e.key) - 1;
        const candidate = TAB_ORDER[idx];
        if (candidate && tabs.includes(candidate)) {
          e.preventDefault();
          onTabChange(candidate);
        }
        return;
      }

      // Cmd/Ctrl + K → open command palette
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
        {/* key={tab} remounts on tab switch to trigger the fade-in animation. */}
        <div key={tab} className="tab-content-enter h-full">
          <TabContent tab={tab} />
        </div>
      </main>
      <Footer />
      <ApprovalModal />
      <CommandPalette open={cmdOpen} onOpenChange={setCmdOpen} />
    </div>
  );
}

function CtxUsage() {
  const tab = useStore((s) => s.tab);
  const contextVisible = useStore((s) => s.chat_context_visible);
  const ctx = useStore((s) => s.chat_context);

  if (tab !== "chat" || !ctx) return null;

  const used = (ctx.message_tokens || 0) + (ctx.tools_tokens || 0);
  const budget = ctx.input_budget || 1;
  const limit = ctx.context_limit || budget;
  const pct = Math.min(100, Math.round((used / budget) * 100));
  const barCls = pct >= 95 ? "err" : pct >= 80 ? "warn" : "";

  // Legacy: topbar mini meter when the context panel is open.
  if (!contextVisible) return null;

  return (
    <div className="ctx-usage" title="LLM context usage">
      <div>
        ctx {formatTokens(used)} / {formatTokens(budget)} of {formatTokens(limit)} (
        {pct}%)
      </div>
      <div className={`token-bar ${barCls}`}>
        <span style={{ width: `${pct}%` }} />
      </div>
    </div>
  );
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
  switch (tab) {
    case "chat":
      return <ChatTab />;
    case "dashboard":
      return <DashboardTab />;
    case "prs":
      return <PrsTab />;
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
        <p className="text-sm">This tab is rendered in the React UI.</p>
        <p className="mt-1 text-xs">
          For the full-featured legacy interface, visit{" "}
          <a href="/legacy" className="text-accent underline">
            /legacy
          </a>
          .
        </p>
      </div>
    </div>
  );
}
