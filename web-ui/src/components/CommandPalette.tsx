import { useState, useMemo, useEffect, useRef } from "react";
import * as Dialog from "@radix-ui/react-dialog";
import { useTheme } from "next-themes";
import { useStore } from "../store/wsStore";
import { apiPost } from "../lib/api";
import {
  MessageSquare,
  LayoutDashboard,
  GitPullRequest,
  Hand,
  ScrollText,
  Settings,
  Plus,
  Trash2,
  Download,
  Sun,
  Moon,
  Zap,
  Search,
  type LucideIcon,
} from "lucide-react";

interface Command {
  id: string;
  label: string;
  hint?: string;
  icon: LucideIcon;
  action: () => void;
  keywords?: string;
}

interface CommandPaletteProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

export default function CommandPalette({ open, onOpenChange }: CommandPaletteProps) {
  const [query, setQuery] = useState("");
  const [selectedIdx, setSelectedIdx] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const { theme, setTheme } = useTheme();

  const tabs = useStore((s) => s.tabs);
  const setTab = useStore((s) => s.setTab);
  const chatBusy = useStore((s) => s.chat_busy);

  const commands = useMemo<Command[]>(() => {
    const close = () => onOpenChange(false);
    const cmds: Command[] = [];

    // Tab switching
    const tabIcons: Record<string, LucideIcon> = {
      chat: MessageSquare,
      dashboard: LayoutDashboard,
      prs: GitPullRequest,
      approvals: Hand,
      logs: ScrollText,
      config: Settings,
    };
    const tabLabels: Record<string, string> = {
      chat: "Chat",
      dashboard: "Dashboard",
      prs: "PRs",
      approvals: "Approvals",
      logs: "Logs",
      config: "Config",
    };
    for (const t of tabs) {
      const Icon = tabIcons[t] ?? MessageSquare;
      cmds.push({
        id: `tab-${t}`,
        label: `Go to ${tabLabels[t] ?? t}`,
        icon: Icon,
        keywords: `switch tab navigate ${t}`,
        action: () => {
          setTab(t);
          void apiPost(`/api/tab/${t}`);
          close();
        },
      });
    }

    // Chat actions
    cmds.push({
      id: "new-session",
      label: "New chat session",
      icon: Plus,
      keywords: "new session chat create",
      action: () => {
        void apiPost("/api/chat/sessions/new");
        close();
      },
    });
    cmds.push({
      id: "clear-chat",
      label: "Clear chat session",
      icon: Trash2,
      keywords: "clear reset delete chat",
      action: () => {
        if (!chatBusy) void apiPost("/api/chat/clear");
        close();
      },
    });
    cmds.push({
      id: "export-chat",
      label: "Export chat transcript",
      icon: Download,
      keywords: "export download save chat markdown",
      action: async () => {
        try {
          const res = await fetch("/api/chat/export");
          if (!res.ok) return;
          const text = await res.text();
          const blob = new Blob([text], { type: "text/markdown" });
          const u = URL.createObjectURL(blob);
          const a = document.createElement("a");
          a.href = u;
          a.download = "chat-transcript.md";
          a.click();
          URL.revokeObjectURL(u);
        } catch {
          /* ignore */
        }
        close();
      },
    });

    // Theme toggle
    cmds.push({
      id: "toggle-theme",
      label: theme === "dark" ? "Switch to light theme" : "Switch to dark theme",
      icon: theme === "dark" ? Sun : Moon,
      keywords: "theme dark light toggle switch color",
      action: () => {
        setTheme(theme === "dark" ? "light" : "dark");
        close();
      },
    });

    // Workflows
    cmds.push({
      id: "run-daily-work",
      label: "Run daily-work workflow",
      icon: Zap,
      keywords: "workflow run daily work digest",
      action: () => {
        void apiPost("/api/workflows/daily-work");
        close();
      },
    });
    cmds.push({
      id: "run-review-radar",
      label: "Run review-radar workflow",
      icon: Zap,
      keywords: "workflow run review radar pr",
      action: () => {
        void apiPost("/api/workflows/review-radar");
        close();
      },
    });

    return cmds;
  }, [tabs, setTab, theme, setTheme, chatBusy, onOpenChange]);

  // Filter by query
  const filtered = useMemo(() => {
    if (!query.trim()) return commands;
    const q = query.toLowerCase();
    return commands.filter(
      (c) =>
        c.label.toLowerCase().includes(q) ||
        (c.keywords?.toLowerCase().includes(q) ?? false),
    );
  }, [commands, query]);

  // Reset on open
  useEffect(() => {
    if (open) {
      setQuery("");
      setSelectedIdx(0);
    }
  }, [open]);

  // Focus input on open
  useEffect(() => {
    if (open) {
      requestAnimationFrame(() => inputRef.current?.focus());
    }
  }, [open]);

  // Clamp selected index
  useEffect(() => {
    if (selectedIdx >= filtered.length) setSelectedIdx(0);
  }, [filtered, selectedIdx]);

  const executeSelected = () => {
    const cmd = filtered[selectedIdx];
    if (cmd) cmd.action();
  };

  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <Dialog.Portal>
        <Dialog.Content
          className="cmd-palette"
          aria-describedby={undefined}
          onOpenAutoFocus={(e) => e.preventDefault()}
        >
          <div className="cmd-palette-box">
            <div className="cmd-palette-input-wrap">
              <Search size={16} className="cmd-palette-search-icon" aria-hidden="true" />
              <input
                ref={inputRef}
                className="cmd-palette-input"
                placeholder="Type a command…"
                value={query}
                onChange={(e) => {
                  setQuery(e.target.value);
                  setSelectedIdx(0);
                }}
                onKeyDown={(e) => {
                  if (e.key === "ArrowDown") {
                    e.preventDefault();
                    setSelectedIdx((i) => Math.min(filtered.length - 1, i + 1));
                  } else if (e.key === "ArrowUp") {
                    e.preventDefault();
                    setSelectedIdx((i) => Math.max(0, i - 1));
                  } else if (e.key === "Enter") {
                    e.preventDefault();
                    executeSelected();
                  }
                }}
              />
              <kbd className="cmd-palette-kbd">Esc</kbd>
            </div>
            {filtered.length > 0 ? (
              <div className="cmd-palette-list">
                {filtered.map((cmd, i) => {
                  const Icon = cmd.icon;
                  return (
                    <button
                      key={cmd.id}
                      type="button"
                      className={`cmd-palette-item${i === selectedIdx ? " is-selected" : ""}`}
                      onMouseEnter={() => setSelectedIdx(i)}
                      onClick={() => cmd.action()}
                    >
                      <Icon size={16} className="cmd-palette-item-icon" aria-hidden="true" />
                      <span className="cmd-palette-item-label">{cmd.label}</span>
                      {cmd.hint && <span className="cmd-palette-item-hint">{cmd.hint}</span>}
                    </button>
                  );
                })}
              </div>
            ) : (
              <div className="cmd-palette-empty">No matching commands</div>
            )}
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
