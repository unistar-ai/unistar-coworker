import { useEffect, useState } from "react";
import { useStore } from "../../store/wsStore";
import { apiFetch, apiPost } from "../../lib/api";
import type { ApprovalRow } from "../../store/protocol";
import {
  approvalKindToToolName,
  buildApprovalPayloadBlocks,
  parseApprovalDescription,
  type ParsedApprovalDescription,
} from "./parser";
import Skeleton from "../../components/Skeleton";
import EmptyState from "../../components/EmptyState";
import { Hand, History, AlertTriangle, Check, Ban } from "lucide-react";

interface HistoryItem {
  id: string;
  kind: string;
  description: string;
  created_at: string;
  decided_at: string | null;
  repo: string | null;
  pr_number: number | null;
  run_id: number | null;
  target_branch: string | null;
  status: string;
  comment_body: string | null;
  issue_number: number | null;
  label: string | null;
}

export default function ApprovalsTab() {
  const [sub, setSub] = useState<"pending" | "history">("pending");
  return (
    <div className="panel">
      <div className="toolbar approval-subtabs">
        <button
          type="button"
          className={`btn btn-ghost${sub === "pending" ? " is-active" : ""}`}
          onClick={() => setSub("pending")}
        >
          Pending
        </button>
        <button
          type="button"
          className={`btn btn-ghost${sub === "history" ? " is-active" : ""}`}
          onClick={() => setSub("history")}
        >
          History
        </button>
      </div>
      {sub === "pending" ? <PendingList /> : <HistoryList />}
    </div>
  );
}

function PendingList() {
  const approvals = useStore((s) => s.approvals);
  const chatPending = useStore((s) => s.chat_pending_approval);
  if (!approvals.length) {
    return (
      <EmptyState
        icon={Hand}
        title="No pending approvals"
        description="Mutating tool calls that need your confirmation will queue up here."
      />
    );
  }
  return (
    <>
      {approvals.map((a) => (
        <ApprovalCard key={a.id} row={a} chatPending={chatPending} />
      ))}
    </>
  );
}

function ApprovalCard({
  row,
  chatPending,
}: {
  row: ApprovalRow;
  chatPending: { id: string; session_id: string; tool_name: string; tool_args_json: string } | null;
}) {
  const toolName = approvalKindToToolName(row.kind);
  const parsed = parseApprovalDescription(row.description, toolName);
  const rejectRecommended = parsed.verdict === "REJECT";
  const toolArgsJson =
    chatPending && chatPending.id === row.id
      ? chatPending.tool_args_json
      : row.comment_body;
  const payloadBlocks = buildApprovalPayloadBlocks(toolName, toolArgsJson);
  const metaParts: string[] = [];
  if (row.repo) metaParts.push(row.repo);
  if (row.pr_number != null) metaParts.push(`#${row.pr_number}`);
  if (row.status) metaParts.push(row.status.replace(/^ApprovalStatus::/, ""));

  return (
    <div className={`approval-card${rejectRecommended ? " verdict-reject" : ""}`}>
      <div className="approval-card-header">
        <h4>{toolName.replace(/_/g, " ")}</h4>
        {metaParts.length > 0 && (
          <div className="approval-card-meta">{metaParts.join(" · ")}</div>
        )}
      </div>
      <ApprovalPayload blocks={payloadBlocks} />
      <ApprovalDetail parsed={parsed} />
      <div className="approval-actions">
        <div className="approval-btn-row">
          <button
            type="button"
            className={`btn ${rejectRecommended ? "btn-primary" : "btn-danger"}`}
            onClick={() => void apiPost(`/api/approvals/${row.id}`, { approve: false })}
          >
            Deny
          </button>
          <button
            type="button"
            className={`btn ${rejectRecommended ? "btn-warn" : "btn-primary"}`}
            onClick={() => void apiPost(`/api/approvals/${row.id}`, { approve: true })}
          >
            {rejectRecommended ? "Approve anyway" : "Approve"}
          </button>
        </div>
      </div>
    </div>
  );
}

function HistoryList() {
  const [items, setItems] = useState<HistoryItem[] | null>(null);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    if (items !== null) return;
    setLoading(true);
    void apiFetch<HistoryItem[]>("/api/approvals/history?limit=50").then((res) => {
      setLoading(false);
      if (res.ok && Array.isArray(res.data)) {
        setItems(res.data);
      } else {
        setItems([]);
      }
    });
  }, [items]);

  if (loading) return <Skeleton rows={4} className="approval-history-skeleton" />;
  if (!items || !items.length) {
    return (
      <EmptyState
        icon={History}
        title="No approval history"
        description="Decided approvals will be listed here once any mutating tool has run."
      />
    );
  }
  return (
    <div className="approval-history-list">
      {items.map((a) => (
        <HistoryItemRow key={a.id} item={a} />
      ))}
    </div>
  );
}

function HistoryItemRow({ item }: { item: HistoryItem }) {
  const toolName = approvalKindToToolName(item.kind);
  const parsed = parseApprovalDescription(item.description, toolName);
  const status = item.status.replace(/^ApprovalStatus::/, "").toLowerCase();
  const when = formatApprovalWhen(item.decided_at || item.created_at);
  const snippet = truncateApprovalSnippet(parsed.summary || item.description);
  const payloadBlocks = buildApprovalPayloadBlocks(toolName, item.comment_body);

  return (
    <details className="approval-history-item">
      <summary className="approval-history-summary">
        <span className={`approval-history-status status-${status}`}>{status}</span>
        <code className="approval-history-tool">{toolName.replace(/_/g, " ")}</code>
        <span className="approval-history-time">{when}</span>
        <span className="approval-history-snippet">{snippet}</span>
      </summary>
      <div className="approval-history-body">
        <ApprovalPayload blocks={payloadBlocks} />
        <ApprovalDetail parsed={parsed} />
      </div>
    </details>
  );
}

function ApprovalPayload({
  blocks,
}: {
  blocks: ReturnType<typeof buildApprovalPayloadBlocks>;
}) {
  if (!blocks.length) return null;
  return (
    <div className="approval-payload">
      {blocks.map((b) => (
        <div key={b.label} className="approval-payload-block">
          <div className="approval-payload-label">{b.label}</div>
          <pre className="approval-payload-pre">{b.text}</pre>
        </div>
      ))}
    </div>
  );
}

function ApprovalDetail({ parsed }: { parsed: ParsedApprovalDescription }) {
  return (
    <div className="approval-detail">
      {parsed.source === "llm-review" && (
        <div
          className={`approval-verdict-banner verdict-${(parsed.verdict || "unknown").toLowerCase()}`}
        >
          <span className="approval-verdict-icon">
            {parsed.verdict === "REJECT" ? <Ban size={16} /> : parsed.verdict === "APPROVE" ? <Check size={16} /> : <AlertTriangle size={16} />}
          </span>
          <div className="approval-verdict-text">
            <strong>LLM safety review · {parsed.verdict || "REVIEW"}</strong>
            <span>
              {parsed.verdict === "REJECT"
                ? "Automated review flagged risks — read before approving."
                : "Review passed — confirm to proceed."}
            </span>
          </div>
        </div>
      )}
      {parsed.issues.length > 1 ? (
        <ul className="approval-issues">
          {parsed.issues.map((issue, i) => (
            <li key={i}>{issue}</li>
          ))}
        </ul>
      ) : (
        parsed.summary && <div className="approval-summary">{parsed.summary}</div>
      )}
    </div>
  );
}

function truncateApprovalSnippet(text: string, max = 120): string {
  const t = (text || "").trim();
  if (t.length <= max) return t;
  return `${t.slice(0, max - 1)}…`;
}

function formatApprovalWhen(ts: string | null): string {
  if (ts == null || ts === "") return "";
  const d = new Date(ts);
  if (Number.isNaN(d.getTime())) return String(ts);
  return d.toLocaleString([], {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  });
}
