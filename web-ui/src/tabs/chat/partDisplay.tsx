import {
  Lightbulb,
  MessageSquare,
  Terminal,
} from "lucide-react";
import {
  formatDurationMs,
  normalizeReasoningText,
  toolMeta,
} from "./parser";
import type { ChatBlock, ToolGroup } from "./parser";
import type { ChatMessagePart } from "./messageParts";
import {
  isShellTool,
  pickToolRowSubtitle,
  subtitleUsesMono,
  toolRowTitle,
} from "./toolDisplay";
import { ToolLucideIcon } from "./toolIcons";

export type DisplayStep = {
  key: string;
  detailBlock: ChatBlock;
  kind: "thought" | "tool" | "comment";
  title: string;
  subtitle?: string;
  status?: string;
  statusKind?: string;
  icon: React.ReactNode;
  toolName?: string;
  /** Subtitle uses monospace (path, command, URL). */
  subtitleMono?: boolean;
  /** Live zone row — spinner + preview styling. */
  isLive?: boolean;
  /** Inline streaming body (live reasoning) instead of block render. */
  liveDetailBody?: string;
};

/** Map collapsed process parts to UI step rows (shared by history + future live parts). */
export function partsToDisplaySteps(
  parts: ChatMessagePart[],
  mcpPrefixes: { id: string; prefix: string }[],
): DisplayStep[] {
  const out: DisplayStep[] = [];
  for (const part of parts) {
    if (part.kind === "reasoning") {
      const body = normalizeReasoningText(part.text);
      const preview = body.replace(/\s+/g, " ").trim();
      pushStep(out, {
        key: part.id,
        detailBlock: {
          type: "reasoning",
          key: part.id,
          reasoningText: part.text,
          reasoningOriginal: part.original,
        },
        kind: "thought",
        title: "深度思考",
        subtitle: preview ? truncate(preview, 100) : undefined,
        icon: <Lightbulb size={15} strokeWidth={2} />,
      });
      continue;
    }
    if (part.kind === "tool-batch") {
      part.groups.forEach((group, i) => {
        const step = describeGroup(part.blockKey, group, mcpPrefixes, i);
        if (step) pushStep(out, step);
      });
      continue;
    }
    if (part.kind === "tool") {
      const step = describeGroup(part.blockKey, part.group, mcpPrefixes, 0);
      if (step) pushStep(out, step);
      continue;
    }
    if (part.kind === "text" && part.role === "assistant") {
      const preview = part.text.replace(/\s+/g, " ").trim();
      pushStep(out, {
        key: part.id,
        detailBlock: {
          type: "message",
          key: part.id,
          message: {
            role: "assistant",
            badge: "",
            body: part.text,
            lineIndex: 0,
            md: part.md ?? false,
          },
        },
        kind: "comment",
        title: "说明",
        subtitle: preview ? truncate(preview, 160) : undefined,
        icon: <MessageSquare size={15} strokeWidth={2} />,
      });
    }
  }
  return out;
}

/** Skip consecutive duplicate tool rows (e.g. replayed ask_user answer). */
function pushStep(out: DisplayStep[], step: DisplayStep) {
  const prev = out[out.length - 1];
  if (
    prev &&
    step.kind === "tool" &&
    prev.kind === "tool" &&
    prev.toolName === step.toolName &&
    prev.subtitle === step.subtitle &&
    prev.statusKind === step.statusKind
  ) {
    return;
  }
  out.push(step);
}

function describeGroup(
  blockKey: string,
  group: ToolGroup,
  mcpPrefixes: { id: string; prefix: string }[],
  index: number,
): DisplayStep | null {
  const meta = toolMeta(group.toolName, mcpPrefixes);
  const commandLine = pickToolRowSubtitle(group);
  const statusKind =
    group.status === "ok"
      ? "ok"
      : group.status === "err"
        ? "err"
        : group.status === "running"
          ? "running"
          : group.status === "pending"
            ? "pending"
            : group.status === "warn"
              ? "warn"
              : "neutral";
  const status = formatStepStatus(group);

  const shell = isShellTool(group.toolName);
  const icon = shell ? (
    <Terminal size={15} strokeWidth={2} />
  ) : (
    <ToolLucideIcon toolName={group.toolName} />
  );

  const title = toolRowTitle(group.toolName, meta.label, group.status);

  return {
    key: `${blockKey}-g${index}`,
    detailBlock: {
      type: "tool-group",
      group,
      key: `${blockKey}-g${index}`,
    },
    kind: "tool",
    title,
    subtitle: commandLine,
    status,
    statusKind,
    icon,
    toolName: group.toolName,
    subtitleMono: commandLine ? subtitleUsesMono(group.toolName) : undefined,
  };
}

function formatStepStatus(group: ToolGroup): string | undefined {
  if (group.status === "ok") {
    const ms = Number.parseInt(String(group.ms ?? "").trim(), 10);
    if (Number.isFinite(ms) && ms >= 0) {
      return ms >= 1000 ? formatDurationMs(ms) : `${ms}ms`;
    }
    return undefined;
  }
  if (group.status === "err") return "失败";
  if (group.status === "pending") return "等待中";
  if (group.status === "running") return "执行中";
  if (group.status === "warn") return "警告";
  return undefined;
}

function truncate(s: string, max: number): string {
  if (s.length <= max) return s;
  return `${s.slice(0, max - 1)}…`;
}

/** Compact preview for collapsed process header (last tools). */
export function formatStepsPeekPreview(steps: DisplayStep[]): string | null {
  const tools = steps.filter((s) => s.kind === "tool");
  if (tools.length === 0) return null;
  const last = tools[tools.length - 1];
  const label = last.subtitle || last.title;
  if (tools.length === 1) return label;
  return `${label} 等 ${tools.length} 项`;
}
