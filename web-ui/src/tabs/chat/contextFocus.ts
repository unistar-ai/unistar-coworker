import { parseContextToolTranscript } from "./contextToolResult";
import type { ToolGroup } from "./parser";

/** Target for deep-linking a chat tool row into the LLM context panel. */
export interface ContextToolFocus {
  toolName: string;
  /** Short args from the chat transcript, e.g. `path=src/lib.rs, max_bytes=1000`. */
  argsShort: string | null;
  /** Leading slice of tool output from the chat UI (when available). */
  outputHint: string | null;
}

/** Match MCP-prefixed tool names in context transcripts. */
export function toolNamesMatch(a: string, b: string): boolean {
  if (a === b) return true;
  return a.endsWith(b) || b.endsWith(a);
}

/** Mirror of Rust `format_tool_args_short` / `format_arg_value` for matching. */
export function formatArgsShortFromRecord(
  args: Record<string, unknown> | null | undefined,
): string {
  if (!args) return "";
  const keys = Object.keys(args);
  if (!keys.length) return "";
  const parts = keys.slice(0, 3).map((key) => `${key}=${formatArgValue(args[key])}`);
  if (keys.length > 3) parts.push("…");
  return parts.join(", ");
}

function formatArgValue(value: unknown): string {
  if (value == null) return "null";
  if (typeof value === "string") return truncateChars(value, 28);
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  return truncateChars(JSON.stringify(value), 28);
}

/** Match Rust `truncate_chars` (append … when clipped). */
function truncateChars(s: string, max: number): string {
  const chars = [...s];
  if (chars.length <= max) return s;
  return `${chars.slice(0, max).join("")}…`;
}

function normalizeWs(s: string): string {
  return s.replace(/\s+/g, " ").trim();
}

/** Strip truncation markers so `foo…` still matches `foobar`. */
function stripEllipsis(s: string): string {
  return s.replace(/[….]+\s*$/u, "").trim();
}

function looseIncludes(hay: string, needle: string): boolean {
  if (!needle) return false;
  if (hay.includes(needle)) return true;
  const a = stripEllipsis(hay);
  const b = stripEllipsis(needle);
  if (!a || !b) return false;
  return a.includes(b) || b.includes(a);
}

function argsMatch(focusArgs: string | null, parsedArgs: Record<string, unknown> | null): boolean {
  const want = normalizeWs(focusArgs || "");
  if (!want) return false;
  const fromJson = formatArgsShortFromRecord(parsedArgs);
  if (
    fromJson &&
    (fromJson === want ||
      looseIncludes(fromJson, want) ||
      looseIncludes(want, fromJson))
  ) {
    return true;
  }
  if (!parsedArgs) return false;
  const pairs = want
    .split(",")
    .map((p) => p.trim())
    .filter((p) => p && p !== "…" && p !== "...");
  if (!pairs.length) return false;
  return pairs.every((pair) => {
    const eq = pair.indexOf("=");
    if (eq <= 0) return false;
    const key = pair.slice(0, eq).trim();
    const val = stripEllipsis(pair.slice(eq + 1).trim());
    if (!(key in parsedArgs)) return false;
    const actualRaw = argValueAsString(parsedArgs[key]);
    const actualShort = formatArgValue(parsedArgs[key]);
    return (
      looseIncludes(actualRaw, val) ||
      looseIncludes(actualShort, val) ||
      looseIncludes(val, actualShort) ||
      looseIncludes(val, actualRaw)
    );
  });
}

function argValueAsString(value: unknown): string {
  if (value == null) return "";
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}

function outputMatch(hint: string | null, body: string): boolean {
  const h = normalizeWs(hint || "");
  if (h.length < 6) return false;
  const b = normalizeWs(body);
  if (!b) return false;
  const needle = h.slice(0, Math.min(96, h.length));
  return looseIncludes(b, needle) || looseIncludes(needle, b.slice(0, Math.min(96, b.length)));
}

/**
 * Pick the context message index for a tool focus target.
 * Returns -1 when there is no confident unique match (avoids jumping to a wrong call).
 */
export function findContextToolMessageIndex(
  messages: { role?: string; content?: string }[],
  focus: ContextToolFocus,
): number {
  type Cand = { index: number; score: number };
  const cands: Cand[] = [];

  for (let i = 0; i < messages.length; i++) {
    const m = messages[i];
    const role = (m.role || "").toLowerCase();
    if (!role.includes("tool")) continue;
    const parsed = parseContextToolTranscript(m.content || "");
    if (!parsed.toolName || !toolNamesMatch(focus.toolName, parsed.toolName)) continue;

    let score = 1; // name only
    if (argsMatch(focus.argsShort, parsed.args)) score += 100;
    if (outputMatch(focus.outputHint, parsed.body)) score += 50;
    // Also try matching hint against the full raw content (legacy callers).
    if (score < 50 && outputMatch(focus.outputHint, m.content || "")) score += 40;
    cands.push({ index: i, score });
  }

  if (!cands.length) return -1;

  cands.sort((a, b) => b.score - a.score || b.index - a.index);
  const best = cands[0];
  // Name-only matches are ambiguous whenever the same tool appears more than once.
  if (best.score <= 1) {
    const nameOnly = cands.filter((c) => c.score <= 1);
    if (nameOnly.length !== 1) return -1;
    return nameOnly[0].index;
  }
  // Require a unique top score among strong matches.
  if (cands.length > 1 && cands[1].score === best.score) return -1;
  return best.index;
}

/** Prefer parsed result body (no tool_result envelope) for context matching. */
export function toolGroupOutputHint(
  steps: { kind: string; output?: string | null }[],
): string | null {
  for (let i = steps.length - 1; i >= 0; i--) {
    const s = steps[i];
    if (s.kind !== "done" || !s.output?.trim()) continue;
    const parsed = parseContextToolTranscript(s.output);
    const body = (parsed.kind !== "plain" ? parsed.body : s.output).trim();
    if (body) return body.slice(0, 200);
    const raw = s.output.trim();
    if (raw) return raw.slice(0, 200);
  }
  return null;
}

/** Build a reliable focus fingerprint from a tool group (full args when available). */
export function buildContextToolFocus(group: ToolGroup): ContextToolFocus {
  let argsShort = (group.args || "").trim() || null;
  let outputHint = toolGroupOutputHint(group.steps);

  for (let i = group.steps.length - 1; i >= 0; i--) {
    const blob = group.steps[i].output?.trim();
    if (!blob) continue;
    const parsed = parseContextToolTranscript(blob);
    if (parsed.args && Object.keys(parsed.args).length > 0) {
      const fromFull = formatArgsShortFromRecord(parsed.args);
      if (fromFull) argsShort = fromFull;
    }
    if (!outputHint && parsed.body.trim()) {
      outputHint = parsed.body.trim().slice(0, 200);
    }
    break;
  }

  return {
    toolName: group.toolName,
    argsShort,
    outputHint,
  };
}
