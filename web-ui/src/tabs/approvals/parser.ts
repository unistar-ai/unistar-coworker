// Approval description parser — mirrors legacy approvals.js::parseApprovalDescription.
// Extracts LLM safety review verdict/summary/issues from the description string.

export interface ParsedApprovalDescription {
  raw: string;
  source: "human" | "llm-review";
  verdict: "APPROVE" | "REJECT" | null;
  summary: string;
  issues: string[];
  toolName: string;
}

export function parseApprovalDescription(
  description: string,
  toolName?: string,
): ParsedApprovalDescription {
  const raw = (description || "").trim();
  let verdict: ParsedApprovalDescription["verdict"] = null;
  let source: ParsedApprovalDescription["source"] = "human";
  let issues: string[] = [];
  let summary = raw;

  const m = raw.match(
    /^Chat:\s*([\w.-]+)\s*[—–-]\s*LLM safety review\s+(APPROVE|REJECT)\s*\(([\s\S]+)\)\s*$/i,
  );
  if (m) {
    source = "llm-review";
    toolName = toolName || m[1];
    verdict = m[2].toUpperCase() as "APPROVE" | "REJECT";
    const body = m[3].trim();
    issues = body
      .split(/\s*;\s*/)
      .map((s) => s.trim())
      .filter(Boolean);
    if (issues.length <= 1 && body.length > 100) {
      issues = body
        .split(/(?<=[。；!！?？])\s+/)
        .map((s) => s.trim())
        .filter(Boolean);
    }
    summary = issues[0] || body;
  } else if (/\bREJECT\b/i.test(raw)) {
    verdict = "REJECT";
    source = "llm-review";
  } else if (/\bAPPROVE\b/i.test(raw)) {
    verdict = "APPROVE";
    source = "llm-review";
  }

  return { raw, source, verdict, summary, issues, toolName: toolName || "tool" };
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
