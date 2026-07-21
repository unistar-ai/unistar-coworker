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
  const approvals = useStore((s) => s.approvals);
  const [pendingId, setPendingId] = useState<string | null>(null);
  const [historyId, setHistoryId] = useState<string | null>(null);

  useEffect(() => {
    if (sub !== "pending") return;
    if (!approvals.length) {
      setPendingId(null);
      return;
    }
    if (!pendingId || !approvals.some((a) => a.id === pendingId)) {
      setPendingId(approvals[0].id);
    }
  }, [sub, approvals, pendingId]);

  return (
    <div className="ops-master-detail">
      <aside className="ops-master-pane">
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
        {sub === "pending" ? (
          <PendingMasterList
            approvals={approvals}
            selectedId={pendingId}
            onSelect={setPendingId}
          />
        ) : (
          <HistoryMasterList selectedId={historyId} onSelect={setHistoryId} />
        )}
      </aside>
      <div className="ops-detail-pane panel">
        {sub === "pending" ? (
          <PendingDetail selectedId={pendingId} />
        ) : (
          <HistoryDetail selectedId={historyId} />
        )}
      </div>
    </div>
  );
}

function PendingMasterList({
  approvals,
  selectedId,
  onSelect,
}: {
  approvals: ApprovalRow[];
  selectedId: string | null;
  onSelect: (id: string) => void;
}) {
  if (!approvals.length) {
    return (
      <p className="config-muted ops-master-empty">暂无待处理</p>
    );
  }
  return (
    <ul className="ops-master-list" role="listbox" aria-label="待处理审批">
      {approvals.map((a) => {
        const toolName = approvalKindToToolName(a.kind);
        const label = toolName.replace(/_/g, " ");
        const active = a.id === selectedId;
        return (
          <li key={a.id}>
            <button
              type="button"
              role="option"
              aria-selected={active}
              className={`ops-master-item${active ? " is-active" : ""}`}
              onClick={() => onSelect(a.id)}
            >
              <span className="ops-master-item-title">{label}</span>
              {a.repo && (
                <span className="ops-master-item-meta">{a.repo}</span>
              )}
            </button>
          </li>
        );
      })}
    </ul>
  );
}

function PendingDetail({ selectedId }: { selectedId: string | null }) {
  const approvals = useStore((s) => s.approvals);
  const chatPending = useStore((s) => s.chat_pending_approval);
  if (!approvals.length) {
    return (
      <EmptyState
        icon={Hand}
        title="No pending approvals"
        description="Mutating tool calls that need confirmation appear here and as a chat modal. Use Approve / Deny in the modal, or /approve /deny."
      />
    );
  }
  const row = approvals.find((a) => a.id === selectedId) ?? approvals[0];
  return (
    <ApprovalCard key={row.id} row={row} chatPending={chatPending} />
  );
}

function HistoryMasterList({
  selectedId,
  onSelect,
}: {
  selectedId: string | null;
  onSelect: (id: string) => void;
}) {
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

  useEffect(() => {
    if (!items?.length) return;
    if (!selectedId || !items.some((i) => i.id === selectedId)) {
      onSelect(items[0].id);
    }
  }, [items, selectedId, onSelect]);

  if (loading) return <Skeleton rows={4} className="approval-history-skeleton" />;
  if (!items?.length) {
    return <p className="config-muted ops-master-empty">暂无历史</p>;
  }

  return (
    <ul className="ops-master-list" role="listbox" aria-label="审批历史">
      {items.map((item) => {
        const toolName = approvalKindToToolName(item.kind);
        const status = item.status.replace(/^ApprovalStatus::/, "").toLowerCase();
        const active = item.id === selectedId;
        return (
          <li key={item.id}>
            <button
              type="button"
              role="option"
              aria-selected={active}
              className={`ops-master-item${active ? " is-active" : ""}`}
              onClick={() => onSelect(item.id)}
            >
              <span className={`approval-history-status status-${status} ops-master-status`}>
                {status}
              </span>
              <span className="ops-master-item-title">{toolName.replace(/_/g, " ")}</span>
              <span className="ops-master-item-meta">
                {formatApprovalWhen(item.decided_at || item.created_at)}
              </span>
            </button>
          </li>
        );
      })}
    </ul>
  );
}

function HistoryDetail({ selectedId }: { selectedId: string | null }) {
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
  if (!items?.length) {
    return (
      <EmptyState
        icon={History}
        title="No approval history"
        description="Decided approvals will be listed here once any mutating tool has run."
      />
    );
  }
  const item = items.find((i) => i.id === selectedId) ?? items[0];
  return <HistoryItemBody item={item} />;
}

function HistoryItemBody({ item }: { item: HistoryItem }) {
  const toolName = approvalKindToToolName(item.kind);
  const parsed = parseApprovalDescription(item.description, toolName);
  const status = item.status.replace(/^ApprovalStatus::/, "").toLowerCase();
  const when = formatApprovalWhen(item.decided_at || item.created_at);
  const payloadBlocks = buildApprovalPayloadBlocks(toolName, item.comment_body);

  return (
    <div className="approval-history-detail">
      <div className="approval-card-header">
        <h4>{toolName.replace(/_/g, " ")}</h4>
        <div className="approval-card-meta">
          <span className={`approval-history-status status-${status}`}>{status}</span>
          {when && <span>{when}</span>}
        </div>
      </div>
      <ApprovalPayload blocks={payloadBlocks} />
      <ApprovalDetail parsed={parsed} />
    </div>
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
    <div className={`approval-card${rejectRecommended ? " verdict-reject" : ""}`} id={`approval-${row.id}`}>
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
