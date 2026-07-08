function clearApprovalArmTimer() {
  if (ui.approvalArmTimer != null) {
    clearTimeout(ui.approvalArmTimer);
    ui.approvalArmTimer = null;
  }
}

function approvalIsArmed(d) {
  return Boolean(d.approve_armed || Date.now() >= ui.approvalArmAt);
}

function scheduleApprovalArmRefresh() {
  clearApprovalArmTimer();
  const wait = Math.max(0, ui.approvalArmAt - Date.now());
  if (wait === 0) return;
  ui.approvalArmTimer = setTimeout(() => {
    ui.approvalArmTimer = null;
    scheduleRender(true);
  }, wait + 5);
}

function syncApprovalArmDeadline(d) {
  if (d.id !== ui.approvalDialogId) {
    ui.approvalDialogId = d.id;
    const ms = Number(d.approve_arm_ms_remaining) || 0;
    ui.approvalArmAt = Date.now() + ms;
    scheduleApprovalArmRefresh();
    return;
  }
  if (!approvalIsArmed(d) && ui.approvalArmTimer == null) {
    scheduleApprovalArmRefresh();
  }
}

function approvalKindToToolName(kind) {
  const k = String(kind || "").replace(/^ApprovalKind::/, "");
  const map = {
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

function resolveApprovalToolArgs(item) {
  if (item?.tool_args_json) return item.tool_args_json;
  if (item?.comment_body) return item.comment_body;
  const pending = state?.chat_pending_approval;
  if (pending && String(pending.id) === String(item?.id)) {
    return pending.tool_args_json || null;
  }
  return null;
}

function parseApprovalArgs(toolName, toolArgsJson) {
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

function appendApprovalPayloadBlock(wrap, label, text) {
  if (text == null || !String(text).trim()) return;
  const block = el("div", "approval-payload-block");
  block.appendChild(el("div", "approval-payload-label", label));
  block.appendChild(el("pre", "approval-payload-pre", String(text)));
  wrap.appendChild(block);
}

function buildApprovalPayload(toolName, toolArgsJson, approval) {
  const wrap = el("div", "approval-payload");
  const name = toolName || "tool";

  if (approval) {
    if (name === "pr_post_comment" && approval.comment_body) {
      appendApprovalPayloadBlock(wrap, "Comment body", approval.comment_body);
      return wrap;
    }
    if (name === "issue_add_label") {
      const parts = [];
      if (approval.repo) parts.push(`repo: ${approval.repo}`);
      if (approval.issue_number != null) parts.push(`issue: #${approval.issue_number}`);
      if (approval.label) parts.push(`label: ${approval.label}`);
      if (parts.length) appendApprovalPayloadBlock(wrap, "Details", parts.join("\n"));
      return wrap;
    }
    if (name === "ci_rerun_workflow" && approval.run_id != null) {
      appendApprovalPayloadBlock(
        wrap,
        "Details",
        `repo: ${approval.repo || "?"}\nrun_id: ${approval.run_id}`,
      );
      return wrap;
    }
    if (name === "pr_create_backport") {
      appendApprovalPayloadBlock(
        wrap,
        "Details",
        `repo: ${approval.repo || "?"}\nPR: #${approval.pr_number ?? "?"}\ntarget: ${approval.target_branch || "?"}`,
      );
      return wrap;
    }
  }

  const info = parseApprovalArgs(name, toolArgsJson);
  if (!info) return wrap;

  const resolvedName = info.toolName || name;
  const args = info.args;

  if (args == null) {
    appendApprovalPayloadBlock(wrap, "Payload", info.raw);
    return wrap;
  }

  switch (resolvedName) {
    case "bash_run":
      appendApprovalPayloadBlock(wrap, "Command", args.command);
      if (args.workdir) appendApprovalPayloadBlock(wrap, "Working directory", args.workdir);
      break;
    case "python_run":
      appendApprovalPayloadBlock(wrap, "Python code", args.code);
      break;
    case "write_file":
      appendApprovalPayloadBlock(wrap, "Path", args.path);
      appendApprovalPayloadBlock(wrap, "Content", args.content);
      break;
    case "edit_file":
      appendApprovalPayloadBlock(wrap, "Path", args.path);
      appendApprovalPayloadBlock(wrap, "Find", args.old_string);
      appendApprovalPayloadBlock(wrap, "Replace with", args.new_string);
      break;
    case "pr_post_comment":
      appendApprovalPayloadBlock(wrap, "Comment body", args.body);
      break;
    case "ci_rerun_workflow":
      appendApprovalPayloadBlock(
        wrap,
        "Details",
        `repo: ${args.repo || "?"}\nrun_id: ${args.run_id ?? "?"}`,
      );
      break;
    case "pr_create_backport":
      appendApprovalPayloadBlock(
        wrap,
        "Details",
        `repo: ${args.repo || "?"}\nPR: #${args.pr_number ?? "?"}\ntarget: ${args.target_branch || "?"}`,
      );
      break;
    case "issue_add_label":
      appendApprovalPayloadBlock(
        wrap,
        "Details",
        `repo: ${args.repo || "?"}\nissue: #${args.issue_number ?? "?"}\nlabel: ${args.label || "?"}`,
      );
      break;
    default:
      appendApprovalPayloadBlock(wrap, "Arguments", JSON.stringify(args, null, 2));
      break;
  }

  if (!wrap.childNodes.length) {
    appendApprovalPayloadBlock(wrap, "Arguments", JSON.stringify(args, null, 2));
  }
  return wrap;
}

function parseApprovalDescription(description, toolName) {
  const raw = (description || "").trim();
  let verdict = null;
  let source = "human";
  let issues = [];
  let summary = raw;

  const m = raw.match(/^Chat:\s*([\w.-]+)\s*[—–-]\s*LLM safety review\s+(APPROVE|REJECT)\s*\(([\s\S]+)\)\s*$/i);
  if (m) {
    source = "llm-review";
    toolName = toolName || m[1];
    verdict = m[2].toUpperCase();
    const body = m[3].trim();
    issues = body.split(/\s*;\s*/).map((s) => s.trim()).filter(Boolean);
    if (issues.length <= 1 && body.length > 100) {
      issues = body.split(/(?<=[。；!！?？])\s+/).map((s) => s.trim()).filter(Boolean);
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

function buildApprovalDescription(parsed) {
  const wrap = el("div", "approval-detail");
  if (parsed.source === "llm-review") {
    const banner = el("div", `approval-verdict-banner verdict-${(parsed.verdict || "unknown").toLowerCase()}`);
    const icon = parsed.verdict === "REJECT" ? "⛔" : parsed.verdict === "APPROVE" ? "✓" : "⚠";
    banner.appendChild(el("span", "approval-verdict-icon", icon));
    const text = el("div", "approval-verdict-text");
    text.appendChild(el("strong", "", `LLM safety review · ${parsed.verdict || "REVIEW"}`));
    text.appendChild(el("span", "", parsed.verdict === "REJECT"
      ? "Automated review flagged risks — read before approving."
      : "Review passed — confirm to proceed."));
    banner.appendChild(text);
    wrap.appendChild(banner);
  }

  if (parsed.issues.length > 1) {
    const list = el("ul", "approval-issues");
    for (const issue of parsed.issues) {
      list.appendChild(el("li", "", issue));
    }
    wrap.appendChild(list);
  } else if (parsed.summary) {
    const body = el("div", "approval-summary", parsed.summary);
    wrap.appendChild(body);
  }

  if (parsed.raw.length > 280 && parsed.issues.length <= 1) {
    const more = el("details", "approval-more");
    more.appendChild(el("summary", "", "Full review text"));
    more.appendChild(el("pre", "approval-raw", parsed.raw));
    wrap.appendChild(more);
  }

  return wrap;
}

function buildApprovalActions(d, armed, deciding, parsed) {
  const actions = el("div", "approval-actions");
  if (deciding) {
    actions.appendChild(el("div", "approval-wait", "Sending decision…"));
    return actions;
  }

  const rejectRecommended = parsed.verdict === "REJECT";
  const hint = el("div", "approval-hint", rejectRecommended
    ? "Deny is recommended when safety review rejected the action."
    : "Mutating action — approve only if you trust this operation.");
  actions.appendChild(hint);

  const row = el("div", "approval-btn-row");
  const no = el("button", `btn ${rejectRecommended ? "btn-primary" : "btn-danger"}`, rejectRecommended ? "Deny (recommended)" : "Deny");
  no.onclick = () => decide(d.id, false);

  const msLeft = Math.max(0, ui.approvalArmAt - Date.now());
  const okLabel = armed
    ? (rejectRecommended ? "Approve anyway" : "Approve")
    : `Approve (${Math.max(1, Math.ceil(msLeft / 50) * 50)}ms)`;
  const ok = el("button", `btn ${rejectRecommended ? "btn-warn" : "btn-primary"}`, okLabel);
  ok.disabled = !armed;
  ok.onclick = () => decide(d.id, true);

  row.append(no, ok);
  actions.appendChild(row);
  return actions;
}

function buildApprovalBox(d, armed, deciding) {
  const parsed = parseApprovalDescription(d.description, d.tool_name);
  const rejectRecommended = parsed.verdict === "REJECT";
  const box = el("div", "approval-box" + (rejectRecommended ? " verdict-reject" : ""));

  const head = el("div", "approval-head");
  head.appendChild(el("div", "approval-head-icon", "⚠"));
  const titles = el("div", "approval-head-text");
  titles.appendChild(el("h3", "", deciding ? "Processing…" : "Approval required"));
  titles.appendChild(el("div", "approval-subtitle", "Mutating tool needs your confirmation"));
  head.appendChild(titles);
  box.appendChild(head);

  const toolRow = el("div", "approval-tool-row");
  toolRow.appendChild(el("span", "approval-tool-label", "Tool"));
  toolRow.appendChild(el("code", "approval-tool-name", parsed.toolName));
  box.appendChild(toolRow);

  const toolArgsJson = resolveApprovalToolArgs(d);
  const payload = buildApprovalPayload(parsed.toolName, toolArgsJson, null);
  if (payload.childNodes.length) box.appendChild(payload);
  box.appendChild(buildApprovalDescription(parsed));
  box.appendChild(buildApprovalActions(d, armed, deciding, parsed));
  return box;
}

function patchApprovalArmButtons(box, armed, parsed) {
  const row = box.querySelector(".approval-btn-row");
  if (!row || row.children.length < 2) return false;
  const no = row.children[0];
  const ok = row.children[1];
  const rejectRecommended = parsed.verdict === "REJECT";
  const msLeft = Math.max(0, ui.approvalArmAt - Date.now());
  const okLabel = armed
    ? (rejectRecommended ? "Approve anyway" : "Approve")
    : `Approve (${Math.max(1, Math.ceil(msLeft / 50) * 50)}ms)`;
  ok.textContent = okLabel;
  ok.disabled = !armed;
  if (no && rejectRecommended) no.textContent = "Deny (recommended)";
  return true;
}

function updateApprovalModal() {
  const d = state?.approval_dialog;
  if (!d) {
    clearApprovalArmTimer();
    ui.approvalDialogId = null;
    ui.approvalArmAt = 0;
    document.querySelectorAll(".approval-modal").forEach((n) => n.remove());
    document.removeEventListener("keydown", onApprovalKeydown);
    return;
  }

  syncApprovalArmDeadline(d);
  const armed = approvalIsArmed(d);
  const deciding = Boolean(d.deciding);
  const parsed = parseApprovalDescription(d.description, d.tool_name);
  const toolArgsJson = resolveApprovalToolArgs(d);
  const stableFp = JSON.stringify({
    id: d.id,
    deciding,
    tool: d.tool_name,
    desc: d.description,
    verdict: parsed.verdict,
    args: toolArgsJson,
  });
  const armFp = JSON.stringify({ armed, ms: Math.max(0, ui.approvalArmAt - Date.now()) });

  let modal = document.querySelector(".approval-modal");
  if (modal?.dataset.stableFp === stableFp && !deciding) {
    const box = modal.querySelector(".approval-box");
    if (box && modal.dataset.armFp !== armFp) {
      patchApprovalArmButtons(box, armed, parsed);
      modal.dataset.armFp = armFp;
    }
    return;
  }

  const fp = stableFp + armFp;
  if (modal?.dataset.fp === fp) return;

  if (!modal) {
    modal = el("div", "approval-modal");
    modal.onclick = (e) => {
      if (e.target === modal && !state.approval_dialog?.deciding) {
        decide(state.approval_dialog.id, false);
      }
    };
    document.body.appendChild(modal);
    document.addEventListener("keydown", onApprovalKeydown);
  }
  modal.dataset.fp = fp;
  modal.dataset.stableFp = stableFp;
  modal.dataset.armFp = armFp;
  modal.replaceChildren(buildApprovalBox(d, armed, deciding));
}

function onApprovalKeydown(e) {
  if (!state?.approval_dialog || state.approval_dialog.deciding) return;
  const d = state.approval_dialog;
  if (e.key === "Escape") {
    e.preventDefault();
    decide(d.id, false);
    return;
  }
  if (e.key === "y" || e.key === "Y") {
    if (!approvalIsArmed(d)) return;
    e.preventDefault();
    decide(d.id, true);
    return;
  }
  if (e.key === "Enter" && !e.shiftKey) {
    if (!approvalIsArmed(d)) return;
    e.preventDefault();
    decide(d.id, true);
  }
}

function approvalStatusLabel(status) {
  return String(status || "")
    .replace(/^ApprovalStatus::/, "")
    .toLowerCase();
}

function truncateApprovalSnippet(text, max = 120) {
  const t = (text || "").trim();
  if (t.length <= max) return t;
  return `${t.slice(0, max - 1)}…`;
}

function formatApprovalWhen(ts) {
  if (ts == null || ts === "") return "";
  const d = typeof ts === "number"
    ? new Date(ts < 1e12 ? ts * 1000 : ts)
    : new Date(ts);
  if (Number.isNaN(d.getTime())) return String(ts);
  return d.toLocaleString([], {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  });
}

function buildApprovalHistorySummary(a) {
  const toolName = approvalKindToToolName(a.kind);
  const parsed = parseApprovalDescription(a.description, toolName);
  const status = approvalStatusLabel(a.status);
  const when = formatApprovalWhen(a.decided_at || a.created_at);
  const snippet = truncateApprovalSnippet(parsed.summary || a.description);

  const details = el("details", "approval-history-item");
  const summary = el("summary", "approval-history-summary");
  summary.appendChild(el("span", `approval-history-status status-${status}`, status));
  summary.appendChild(el("code", "approval-history-tool", toolName.replace(/_/g, " ")));
  summary.appendChild(el("span", "approval-history-time", when));
  summary.appendChild(el("span", "approval-history-snippet", snippet));
  details.appendChild(summary);

  const body = el("div", "approval-history-body");
  const payload = buildApprovalPayload(toolName, resolveApprovalToolArgs(a), a);
  if (payload.childNodes.length) body.appendChild(payload);
  body.appendChild(buildApprovalDescription(parsed));
  details.appendChild(body);
  return details;
}
