// Approval description parser — mirrors legacy approvals.js::parseApprovalDescription.
// Extracts LLM safety review verdict/summary/issues from the description string.

export interface ParsedApprovalDescription {
  raw: string;
  source: "human" | "llm-review";
  verdict: "APPROVE" | "REJECT" | null;
  summary: string;
  issues: string[];
  /**
   * Lines for the Reason block (always preferred over raw description dump).
   * - llm-review: safety-review body / issues
   * - human: why confirmation is needed (not the `Chat:` action title)
   */
  reasonLines: string[];
  toolName: string;
}

const LLM_REVIEW_DESC_RE =
  /^Chat:\s*([\w.-]+)\s*[—–-]\s*LLM safety review\s+(APPROVE|REJECT)\s*\(([\s\S]+)\)\s*$/i;

export function parseApprovalDescription(
  description: string,
  toolName?: string,
): ParsedApprovalDescription {
  const raw = (description || "").trim();
  let verdict: ParsedApprovalDescription["verdict"] = null;
  let source: ParsedApprovalDescription["source"] = "human";
  let issues: string[] = [];
  let summary = raw;

  const m = raw.match(LLM_REVIEW_DESC_RE);
  if (m) {
    source = "llm-review";
    toolName = toolName || m[1];
    verdict = m[2].toUpperCase() as "APPROVE" | "REJECT";
    const body = m[3].trim();
    issues = splitApprovalReasonBody(body);
    summary = issues[0] || normalizeApprovalIssueText(body);
  } else if (/LLM safety review\s+REJECT/i.test(raw)) {
    verdict = "REJECT";
    source = "llm-review";
    summary = raw;
  } else if (/LLM safety review\s+APPROVE/i.test(raw)) {
    verdict = "APPROVE";
    source = "llm-review";
    summary = raw;
  }

  const reasonLines = buildApprovalReasonLines(
    raw,
    source,
    issues,
    summary,
    toolName || "tool",
  );
  return {
    raw,
    source,
    verdict,
    summary,
    issues,
    reasonLines,
    toolName: toolName || "tool",
  };
}

/** Lines for the Reason heading — always non-empty when description exists. */
export function approvalReasonLines(parsed: ParsedApprovalDescription): string[] {
  const base =
    parsed.reasonLines.length > 0
      ? parsed.reasonLines
      : buildApprovalReasonLines(
          parsed.raw,
          parsed.source,
          parsed.issues,
          parsed.summary,
          parsed.toolName,
        );
  return expandReasonDisplayLines(base);
}

/** Split dense review blobs into readable bullets (`;`, `#1`/`#2`, numbered lists). */
export function expandReasonDisplayLines(lines: string[]): string[] {
  const out: string[] = [];
  for (const raw of lines) {
    const line = raw.trim();
    if (!line) continue;
    const bySemi = line
      .split(/\s*;\s*/)
      .map((s) => s.trim())
      .filter(Boolean);
    const chunks = bySemi.length > 1 ? bySemi : [line];
    for (const chunk of chunks) {
      const numbered = chunk
        .split(/(?=(?:^|\s)#\d+\b)|(?=(?:^|\s)\d+\.\s+)/)
        .map((s) => s.trim())
        .filter(Boolean);
      if (numbered.length > 1) {
        out.push(...numbered.map(stripLeadingEnumMarker));
      } else {
        out.push(stripLeadingEnumMarker(chunk));
      }
    }
  }
  // Dedupe consecutive identical lines
  return out.filter((line, i) => line && line !== out[i - 1]);
}

function stripLeadingEnumMarker(s: string): string {
  return s.replace(/^(?:#\d+\b|\d+\.)\s*/, "").trim();
}

export type ReasonDisplayItem =
  | { kind: "issue"; riskType: string | null; text: string }
  | { kind: "suggestion"; text: string };

/** Classify reason lines into primary issues vs follow-up suggestions. */
export function classifyReasonDisplayItems(lines: string[]): ReasonDisplayItem[] {
  const expanded = expandReasonDisplayLines(lines);
  if (expanded.length === 0) return [];

  const items: ReasonDisplayItem[] = [];
  let sawPrimary = false;
  for (const line of expanded) {
    const risk = line.match(
      /^(HIGH_RISK_COMMAND|SECURITY_VULNERABILITY|AI_HALLUCINATION|MISSING_ERROR_HANDLING)\s*:\s*([\s\S]+)$/i,
    );
    if (risk) {
      items.push({
        kind: "issue",
        riskType: risk[1].toUpperCase(),
        text: risk[2].trim(),
      });
      sawPrimary = true;
      continue;
    }
    if (!sawPrimary) {
      items.push({ kind: "issue", riskType: null, text: line });
      sawPrimary = true;
    } else {
      items.push({ kind: "suggestion", text: line });
    }
  }
  return items;
}

function buildApprovalReasonLines(
  raw: string,
  source: ParsedApprovalDescription["source"],
  issues: string[],
  summary: string,
  toolName: string,
): string[] {
  if (source === "llm-review") {
    const fromIssues = issues.map((s) => s.trim()).filter(Boolean);
    if (fromIssues.length > 0) return fromIssues;

    const sum = summary.trim();
    const inner = sum.match(LLM_REVIEW_DESC_RE) || raw.match(LLM_REVIEW_DESC_RE);
    if (inner) {
      const body = normalizeApprovalIssueText(inner[3].trim());
      if (body) return [body];
    }

    const stripped = sum
      .replace(
        /^Chat:\s*[\w.-]+\s*[—–-]\s*LLM safety review\s+(APPROVE|REJECT)\s*/i,
        "",
      )
      .trim();
    if (stripped) return [normalizeApprovalIssueText(stripped)].filter(Boolean);
    if (sum) return [normalizeApprovalIssueText(sum)].filter(Boolean);
    return [];
  }

  // Harness / MCP: `description` is an internal Chat: title that duplicates DETAILS.
  // Reason explains *why* approval is required, not the payload again.
  return [humanApprovalReason(toolName, raw)];
}

/** Why this mutating action needs confirmation (not a Chat: action title). */
function humanApprovalReason(toolName: string, raw: string): string {
  const name = (toolName || "").trim() || "tool";
  if (/^Chat:\s*MCP\b/i.test(raw) || name === "mcp_tool" || name.includes("__")) {
    return "MCP mutating tool — confirm before it can change remote state.";
  }
  switch (name) {
    case "issue_add_label":
      return "Adding a GitHub issue label writes to the remote and needs your confirmation.";
    case "pr_post_comment":
      return "Posting a PR comment writes to GitHub and needs your confirmation.";
    case "ci_rerun_workflow":
      return "Re-running a workflow triggers CI on GitHub and needs your confirmation.";
    case "pr_create_backport":
      return "Creating a backport PR writes to GitHub and needs your confirmation.";
    case "bash_run":
    case "python_run":
    case "write_file":
    case "edit_file":
      return "This workspace action was escalated for human confirmation before running.";
    default:
      return `Mutating tool \`${name}\` needs your confirmation before it runs.`;
  }
}

function splitApprovalReasonBody(body: string): string[] {
  let parts = body
    .split(/\s*;\s*/)
    .map((s) => normalizeApprovalIssueText(s.trim()))
    .filter(Boolean);

  if (parts.length === 0 && body.length > 0) {
    parts = [normalizeApprovalIssueText(body)];
  } else if (parts.length === 1 && body.length > 100) {
    const sentences = body
      .split(/(?<=[。；!！?？])\s+/)
      .map((s) => normalizeApprovalIssueText(s.trim()))
      .filter(Boolean);
    if (sentences.length > 1) {
      parts = sentences;
    }
  }
  return parts;
}

/** Legacy approvals stored `reason=<command>` when the reviewer omitted issue text. */
function normalizeApprovalIssueText(text: string): string {
  if (!text) return "";
  const reasonMatch = text.match(/^reason=(.+)$/i);
  if (reasonMatch) {
    const val = reasonMatch[1].trim();
    if (
      val.length > 72 ||
      /^(gh |export |curl |sudo )/i.test(val) ||
      val.includes(" && ") ||
      val.includes("|")
    ) {
      return "Automated safety check flagged this command — see COMMAND above.";
    }
    return val;
  }
  return text;
}

/** Map an ApprovalKind enum string to a friendly tool name. */
export function approvalKindToToolName(kind: string): string {
  const k = String(kind || "").replace(/^ApprovalKind::/, "");
  const map: Record<string, string> = {
    BashRun: "bash_run",
    PythonRun: "python_run",
    WriteFile: "write_file",
    EditFile: "edit_file",
    RerunFlaky: "ci_rerun_workflow",
    Backport: "pr_create_backport",
    PostComment: "pr_post_comment",
    IssueAddLabel: "issue_add_label",
    McpTool: "mcp_tool",
  };
  return map[k] || k.replace(/([a-z])([A-Z])/g, "$1_$2").toLowerCase();
}

/** Parse tool_args_json into { toolName, args, raw }. */
export function parseApprovalArgs(
  toolName: string,
  toolArgsJson: string | null,
): { toolName: string; args: unknown; raw: string } | null {
  if (!toolArgsJson || !String(toolArgsJson).trim()) return null;
  const raw = String(toolArgsJson).trim();
  try {
    const parsed = JSON.parse(raw);
    if (parsed && typeof parsed === "object" && parsed.tool_name && parsed.args != null) {
      return { toolName: parsed.tool_name, args: parsed.args, raw };
    }
    return { toolName: toolName || "tool", args: parsed, raw };
  } catch {
    return { toolName: toolName || "tool", args: null, raw };
  }
}

export interface PayloadBlock {
  label: string;
  text: string;
}

/** Build a list of (label, text) payload blocks based on tool type. */
export function buildApprovalPayloadBlocks(
  toolName: string,
  toolArgsJson: string | null,
): PayloadBlock[] {
  const name = toolName || "tool";
  const info = parseApprovalArgs(name, toolArgsJson);
  if (!info) return [];
  const blocks: PayloadBlock[] = [];
  const args = info.args as Record<string, unknown> | null;
  const resolvedName = info.toolName || name;

  if (args == null) {
    return [{ label: "Payload", text: info.raw }];
  }

  const add = (label: string, text: unknown) => {
    if (text != null && String(text).trim()) {
      blocks.push({ label, text: String(text) });
    }
  };

  switch (resolvedName) {
    case "bash_run":
      add("Command", args.command);
      add("Working directory", args.workdir);
      break;
    case "python_run":
      add("Python code", args.code);
      break;
    case "write_file":
      add("Path", args.path);
      add("Content", args.content);
      break;
    case "edit_file":
      add("Path", args.path);
      add("Find", args.old_string);
      add("Replace with", args.new_string);
      break;
    case "pr_post_comment":
      add("Comment body", args.body);
      break;
    case "ci_rerun_workflow":
      add("Details", `repo: ${args.repo || "?"}\nrun_id: ${args.run_id ?? "?"}`);
      break;
    case "pr_create_backport":
      add(
        "Details",
        `repo: ${args.repo || "?"}\nPR: #${args.pr_number ?? "?"}\ntarget: ${args.target_branch || "?"}`,
      );
      break;
    case "issue_add_label":
      add(
        "Details",
        `repo: ${args.repo || "?"}\nissue: #${args.issue_number ?? "?"}\nlabel: ${args.label || "?"}`,
      );
      break;
    default:
      add("Arguments", JSON.stringify(args, null, 2));
      break;
  }
  if (blocks.length === 0) {
    add("Arguments", JSON.stringify(args, null, 2));
  }
  return blocks;
}
