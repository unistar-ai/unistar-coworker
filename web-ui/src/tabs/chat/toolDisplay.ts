import {
  parseContextToolTranscript,
  type ParsedToolTranscript,
  type ToolTranscriptKind,
} from "./contextToolResult";
import {
  isLongToolArgKey,
  parseToolArgsString,
  type ArgPair,
  type ToolGroup,
  type ToolStep,
} from "./parser";

const PRIMARY_ARG_KEYS: Record<string, string[]> = {
  bash_run: ["command", "cmd"],
  python_run: ["code"],
  read_file: ["path"],
  write_file: ["path"],
  edit_file: ["path"],
  grep: ["pattern", "path"],
  glob: ["pattern", "path"],
  web_fetch: ["url"],
  web_browser: ["url"],
  skill_load: ["name"],
  skill_search: ["query"],
  ask_user: ["question"],
  tool_search: ["query"],
  tool_call: ["name"],
  pr_get_diff: ["repo", "pr_number", "path"],
  pr_get_overview: ["repo", "pr_number"],
  pr_list_changed_files: ["repo", "pr_number"],
  pr_list_open: ["repo"],
  pr_list_merged: ["repo"],
  pr_list_waiting_review: ["repo"],
  pr_diff_risk_scan: ["repo", "pr_number"],
  pr_get_ci_snapshot: ["repo", "pr_number"],
  pr_get_review_routing: ["repo", "pr_number"],
  pr_get_review_state: ["repo", "pr_number"],
};

export type ToolBodyDisplay =
  | "markdown"
  | "ask_user"
  | "shell"
  | "summarized"
  | "read_file"
  | "grep"
  | "pre";

export interface PreparedToolStepDisplay {
  parsed: ParsedToolTranscript;
  body: string;
  display: ToolBodyDisplay;
  error: boolean;
}

export function isShellTool(toolName: string | null | undefined): boolean {
  return toolName === "bash_run" || toolName === "python_run";
}

export function toolRowTitle(
  toolName: string,
  label: string,
  status: ToolGroup["status"],
): string {
  if (isShellTool(toolName)) return "执行命令";
  if (toolName === "read_file") return "读取文件";
  if (toolName === "write_file") return "写入文件";
  if (toolName === "edit_file") return "编辑文件";
  if (toolName === "grep") return "搜索代码";
  if (toolName === "glob") return "查找文件";
  if (toolName === "web_fetch" || toolName === "web_browser") return "获取网页";
  if (toolName === "skill_load") return "加载技能";
  if (toolName === "skill_search") return "搜索技能";
  if (toolName === "tool_search") return "搜索工具";
  if (toolName === "ask_user") {
    return status === "pending" ? "等待你的回答" : "向用户提问";
  }
  return label;
}

function truncate(s: string, max: number): string {
  if (s.length <= max) return s;
  return `${s.slice(0, max - 1)}…`;
}

function argValue(pairs: ArgPair[], ...keys: string[]): string | null {
  for (const key of keys) {
    const hit = pairs.find((p) => p.key === key)?.value?.trim();
    if (hit) return hit;
  }
  return null;
}

/** Human-readable subtitle from tool arguments. */
export function toolArgSubtitle(
  toolName: string,
  args: string | null | undefined,
): string | null {
  if (!args?.trim()) return null;
  const pairs = parseToolArgsString(args);
  const keys = PRIMARY_ARG_KEYS[toolName];
  if (keys) {
    if (toolName === "grep") {
      const pattern = argValue(pairs, "pattern");
      const path = argValue(pairs, "path");
      if (pattern && path) return truncate(`${pattern} · ${path}`, 160);
      if (pattern) return truncate(pattern, 160);
    }
    if (toolName.startsWith("pr_")) {
      const repo = argValue(pairs, "repo");
      const pr = argValue(pairs, "pr_number", "pr");
      const path = argValue(pairs, "path");
      const bits = [repo, pr ? `#${pr}` : null, path].filter(Boolean);
      if (bits.length) return truncate(bits.join(" · "), 160);
    }
    for (const key of keys) {
      const val = argValue(pairs, key);
      if (val) return truncate(val, toolName === "bash_run" ? 200 : 160);
    }
  }
  const first = pairs[0];
  if (!first) return truncate(args.trim(), 120);
  if (!first.value) return first.key;
  return truncate(`${first.key}=${first.value}`, 160);
}

function previewFromParsedBody(toolName: string, body: string): string | null {
  const blob = body.trim();
  if (!blob) return null;

  if (toolName === "ask_user") {
    if (/awaiting user answer/i.test(blob)) {
      return blob.match(/Question:\s*(.+?)(?:\n|$)/)?.[1]?.trim() || null;
    }
    const ans = blob.match(/User answered:\s*\n?([\s\S]+)/i)?.[1];
    if (ans?.trim()) return ans.trim().split("\n")[0].trim();
  }

  if (toolName === "skill_load") {
    return blob.match(/###\s+(\S+)/)?.[1] || null;
  }

  if (isShellTool(toolName)) {
    const shell = parseShellTranscript(blob);
    if (shell?.stdout?.trim()) {
      const line = shell.stdout.trim().split("\n")[0].trim();
      if (line) return truncate(line, 160);
    }
    if (shell?.exit) return shell.exit;
  }

  if (toolName === "read_file") {
    const firstLine = blob.split("\n")[0]?.trim() || "";
    const header = firstLine.match(/^path:\s*(.+?)(?:\s*\(|$)/)?.[1];
    if (header) return truncate(header.trim(), 160);
  }

  if (toolName === "grep") {
    const header = blob.match(/^grep `([^`]+)`/);
    if (header) return truncate(header[1], 120);
    if (/no matches/i.test(blob)) return "无匹配";
  }

  if (toolName === "glob") {
    const lines = blob.split("\n").filter((l) => l.trim() && !l.startsWith("["));
    if (lines.length === 1) return truncate(lines[0], 160);
    if (lines.length > 1) return `${lines.length} 个文件`;
  }

  if (toolName === "web_fetch" || toolName === "web_browser") {
    const ok = blob.match(/^OK:\s*(\d+)/)?.[1];
    if (ok) return `HTTP ${ok}`;
    const title = blob.match(/^#\s+(.+)/m)?.[1];
    if (title) return truncate(title, 140);
  }

  if (toolName === "write_file" || toolName === "edit_file") {
    if (/^wrote\b/i.test(blob)) return blob.split("\n")[0].trim();
    if (/^diff --git/m.test(blob)) return "diff";
  }

  if (parsedKindUsesMarkdown(toolName, blob)) {
    const heading = blob.match(/^#{1,3}\s+(.+)/m)?.[1];
    if (heading) return truncate(heading, 140);
  }

  const line = blob
    .split("\n")
    .find((l) => {
      const t = l.trim();
      if (!t) return false;
      if (/^args:/i.test(t)) return false;
      if (/^tool_result\(/i.test(t)) return false;
      if (/^review:/i.test(t)) return false;
      if (/^cwd:/i.test(t)) return false;
      if (/^exit:/i.test(t)) return false;
      return true;
    });
  return line ? truncate(line.trim(), 160) : null;
}

/** Short summary for collapsed tool chips / cards. */
export function toolCollapsedSummary(group: ToolGroup): string | null {
  const subtitle = pickToolRowSubtitle(group);
  if (subtitle) return subtitle;
  for (const step of [...group.steps].reverse()) {
    const blob = step.output?.trim();
    if (!blob) continue;
    const prepared = prepareToolStepDisplay(group.toolName, blob);
    const lines = prepared.body.split("\n").filter((l) => l.trim()).length;
    if (lines > 1) return `${lines} 行`;
    if (prepared.body.length > 96) return `${prepared.body.length} 字符`;
  }
  return null;
}

/** Whether row subtitle should use monospace (paths, commands, URLs). */
export function subtitleUsesMono(toolName: string): boolean {
  return (
    isShellTool(toolName) ||
    toolName === "read_file" ||
    toolName === "write_file" ||
    toolName === "edit_file" ||
    toolName === "grep" ||
    toolName === "glob" ||
    toolName === "web_fetch" ||
    toolName === "web_browser" ||
    toolName.startsWith("pr_")
  );
}

/** Preview line from raw step output (strips LLM transcript envelope). */
export function toolOutputPreview(toolName: string, rawOutput: string): string | null {
  const parsed = parseContextToolTranscript(rawOutput);
  const body = parsed.kind !== "plain" ? parsed.body : rawOutput;
  return previewFromParsedBody(toolName, body);
}

/** Preview from tool step list — used by process row subtitles. */
export function toolGroupOutputPreview(
  toolName: string,
  steps: ToolStep[],
): string | null {
  for (const step of [...steps].reverse()) {
    const blob = step.output?.trim();
    if (!blob) continue;
    const preview = toolOutputPreview(toolName, blob);
    if (preview) return preview;
  }
  return null;
}

/** Row subtitle: prefer args for in-flight tools, output preview when settled. */
export function pickToolRowSubtitle(group: ToolGroup): string | undefined {
  const fromArgs = toolArgSubtitle(group.toolName, group.args);
  const fromOutput = toolGroupOutputPreview(group.toolName, group.steps);

  let line: string | null = null;
  if (group.toolName === "ask_user") {
    line = fromOutput || fromArgs;
    if (line) line = line.replace(/^User answered:\s*/i, "").trim();
  } else if (group.status === "running" || group.status === "pending") {
    line = fromArgs || fromOutput;
  } else if (
    isShellTool(group.toolName) ||
    group.toolName === "grep" ||
    group.toolName === "read_file" ||
    group.toolName === "write_file" ||
    group.toolName === "edit_file"
  ) {
    line = fromArgs || fromOutput;
  } else {
    line = fromOutput || fromArgs;
  }

  if (!line?.trim()) return undefined;
  return truncate(line.trim(), 200);
}

export function parsedKindUsesMarkdown(
  toolName: string | null,
  body: string,
): boolean {
  const t = body.trimStart();
  if (!t) return false;
  if (toolName === "skill_load" || toolName === "skill_search") return true;
  if (toolName === "web_fetch" || toolName === "web_browser") return /^#\s+/m.test(t);
  if (/^#{1,3}\s/.test(t)) return true;
  return false;
}

export interface ShellTranscriptParts {
  review?: string;
  cwd?: string;
  exit?: string;
  stdout?: string;
  stderr?: string;
}

export function parseShellTranscript(body: string): ShellTranscriptParts | null {
  if (
    !/^(review:|cwd:|exit:|stdout:|stderr:)/m.test(body) &&
    !/^exit:\s*\d/m.test(body)
  ) {
    return null;
  }
  const parts: ShellTranscriptParts = {};
  const lines = body.split("\n");
  let section: "meta" | "stdout" | "stderr" = "meta";
  const stdout: string[] = [];
  const stderr: string[] = [];

  for (const line of lines) {
    const trimmed = line.trim();
    if (section === "meta") {
      if (/^stdout:\s*$/i.test(line)) {
        section = "stdout";
        continue;
      }
      if (/^stderr:\s*$/i.test(line)) {
        section = "stderr";
        continue;
      }
      if (/^review:/i.test(line)) {
        parts.review = line.replace(/^review:\s*/i, "").trim();
        continue;
      }
      if (/^cwd:/i.test(line)) {
        parts.cwd = line.replace(/^cwd:\s*/i, "").trim();
        continue;
      }
      if (/^exit:/i.test(line)) {
        parts.exit = line.replace(/^exit:\s*/i, "").trim();
        continue;
      }
      if (/^(bash_run|python_run)\b/i.test(trimmed)) continue;
      continue;
    }
    if (section === "stdout") {
      if (/^stderr:\s*$/i.test(line)) {
        section = "stderr";
        continue;
      }
      stdout.push(line);
      continue;
    }
    stderr.push(line);
  }

  if (stdout.length) parts.stdout = stdout.join("\n").trimEnd();
  if (stderr.length) parts.stderr = stderr.join("\n").trimEnd();
  if (
    !parts.review &&
    !parts.cwd &&
    !parts.exit &&
    !parts.stdout &&
    !parts.stderr
  ) {
    return null;
  }
  return parts;
}

function stripRedundantBodyHeader(
  toolName: string | null,
  body: string,
  args: Record<string, unknown> | null,
): string {
  let out = body;
  if (toolName === "read_file") {
    out = out.replace(/^path:\s*.+?\n/, "");
  }
  if (toolName === "grep") {
    out = out.replace(/^grep `[^`]+` \([^)]+\):\n/, "");
    const pattern = typeof args?.pattern === "string" ? args.pattern : null;
    if (pattern) {
      out = out.replace(new RegExp(`^grep \`${escapeRegExp(pattern)}\`[^\n]*\n`), "");
    }
  }
  return out.trimStart();
}

function escapeRegExp(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

export function prepareToolStepDisplay(
  toolName: string | null | undefined,
  rawOutput: string,
): PreparedToolStepDisplay {
  const parsed = parseContextToolTranscript(rawOutput);
  const effectiveName = toolName ?? parsed.toolName;
  let body = parsed.kind !== "plain" ? parsed.body : rawOutput;
  const error = parsed.kind === "error" || parsed.ok === false;

  if (parsed.kind === "summarized") {
    return { parsed, body: body.trim(), display: "summarized", error: false };
  }

  if (effectiveName === "ask_user" || parsed.kind === "ask_user") {
    return { parsed, body, display: "ask_user", error };
  }

  body = stripRedundantBodyHeader(effectiveName, body, parsed.args);

  if (effectiveName === "read_file" && /^\d+\|/m.test(body)) {
    return { parsed, body, display: "read_file", error };
  }

  if (effectiveName === "grep") {
    return { parsed, body, display: "grep", error };
  }

  if (isShellTool(effectiveName) && parseShellTranscript(body)) {
    return { parsed, body, display: "shell", error };
  }

  if (parsedKindUsesMarkdown(effectiveName, body)) {
    return { parsed, body, display: "markdown", error };
  }

  return { parsed, body, display: "pre", error };
}

function argJsonValueToString(value: unknown): string {
  if (value == null) return "";
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}

/** Convert transcript JSON args into display pairs (full values, no truncation). */
export function argPairsFromRecord(
  args: Record<string, unknown> | null | undefined,
): ArgPair[] {
  if (!args) return [];
  return Object.entries(args).map(([key, value]) => ({
    key,
    value: argJsonValueToString(value),
  }));
}

/**
 * Prefer full args embedded in tool_result / tool_error transcripts.
 * Line-level `group.args` come from `format_tool_args_short` and are truncated.
 */
export function resolveToolArgPairs(group: ToolGroup): ArgPair[] {
  for (const step of [...group.steps].reverse()) {
    const blob = step.output?.trim();
    if (!blob) continue;
    const parsed = parseContextToolTranscript(blob);
    const pairs = argPairsFromRecord(parsed.args);
    if (pairs.length > 0) return pairs;
  }
  const fromLine = parseToolArgsString(group.args);
  if (fromLine.length > 0) return fromLine;
  const raw = group.args?.trim();
  if (!raw) return [];
  return [{ key: "args", value: raw }];
}

/** Values long enough (or multi-line) that the params pane should use a `<pre>` block. */
export function preferArgBlock(key: string, value: string | undefined): boolean {
  if (!value) return false;
  if (isLongToolArgKey(key)) return true;
  if (value.includes("\n")) return true;
  return value.length > 72;
}

/** Hide arg chips in inline detail when parent row already shows the same values. */
export function shouldShowInlineArgChips(
  chipArgs: ArgPair[],
  rowSubtitle?: string,
): boolean {
  if (chipArgs.length === 0) return false;
  const sub = (rowSubtitle || "").toLowerCase();
  if (!sub) return true;
  return chipArgs.some((p) => {
    if (!p.value?.trim()) return true;
    const val = p.value.trim().toLowerCase();
    if (val.length <= 3) return true;
    return !sub.includes(val.slice(0, Math.min(val.length, 24)));
  });
}

/** Hide command/code block when the collapsed row already shows it. */
export function shouldShowInlineCommandBlock(
  toolName: string,
  blockArgs: ArgPair[],
  rowSubtitle?: string,
): boolean {
  if (blockArgs.length === 0) return false;
  if (!isShellTool(toolName)) return true;
  const cmd = blockArgs.find((p) => p.key === "command" || p.key === "code")?.value;
  if (!cmd?.trim() || !rowSubtitle) return true;
  const normCmd = cmd.trim().replace(/\s+/g, " ");
  const normSub = rowSubtitle.trim().replace(/\s+/g, " ");
  return !normSub.includes(normCmd.slice(0, Math.min(normCmd.length, 48)));
}

export function parseAskUserBody(body: string): {
  question?: string;
  options: string[];
  answer?: string;
  pending: boolean;
} {
  const pending = /awaiting user answer/i.test(body);
  const question = body.match(/Question:\s*(.+?)(?:\n|$)/)?.[1]?.trim();
  const answer = body.match(/User answered:\s*\n?([\s\S]+)/i)?.[1]?.trim();
  const options: string[] = [];
  const optBlock = body.match(/Options:\s*\n([\s\S]*?)(?:\n\n|$)/i)?.[1];
  if (optBlock) {
    for (const line of optBlock.split("\n")) {
      const m = line.match(/^\s*\d+\.\s+(.+)/);
      if (m) options.push(m[1].trim());
    }
  }
  return { question, options, answer, pending };
}

export function transcriptKindLabel(kind: ToolTranscriptKind): string | null {
  switch (kind) {
    case "error":
      return "失败";
    case "approval":
      return "等待批准";
    case "ask_user":
      return "等待回答";
    case "summarized":
      return "已摘要";
    default:
      return null;
  }
}
