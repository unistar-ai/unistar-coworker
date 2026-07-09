import { useStore } from "../../store/wsStore";
import { apiPost } from "../../lib/api";
import Markdown from "../../components/Markdown";
import EmptyState from "../../components/EmptyState";
import { ClipboardList, FileText } from "lucide-react";
import type { DigestSummary } from "../../store/protocol";

export default function DashboardTab() {
  const history = useStore((s) => s.digest_history);
  const bodies = useStore((s) => s.digest_bodies);
  const selectedDate = useStore((s) => s.selected_digest_date);

  const activeDate = selectedDate || history[0]?.date;

  return (
    <div className="split-panel">
      <div className="split-list">
        <div className="toolbar">
          <button
            type="button"
            className="btn btn-ghost"
            onClick={() => void apiPost("/api/store/refresh")}
          >
            Refresh store
          </button>
        </div>
        {history.length === 0 ? (
          <EmptyState
            icon={ClipboardList}
            title="No digests yet"
            description="Historical digests from older versions may still appear here. New digests are not generated automatically — use chat or triage-pr for GitHub ops."
          />
        ) : (
          <ul className="list">
            {history.map((d, i) => (
              <DigestRow
                key={d.date}
                d={d}
                active={d.date === activeDate}
                preview={bodies[d.date] || ""}
                onSelect={() => void apiPost(`/api/digest/${i}/select`)}
              />
            ))}
          </ul>
        )}
      </div>
      <div className="split-detail">
        {activeDate && bodies[activeDate] ? (
          <Markdown>{bodies[activeDate]}</Markdown>
        ) : (
          <EmptyState
            icon={FileText}
            title={history.length ? "Select a digest" : "Nothing to show yet"}
            description={
              history.length
                ? "Pick a date on the left to read its digest."
                : "No digest content in the local store."
            }
          />
        )}
      </div>
    </div>
  );
}

function DigestRow({
  d,
  active,
  preview,
  onSelect,
}: {
  d: DigestSummary;
  active: boolean;
  preview: string;
  onSelect: () => void;
}) {
  const summary = preview
    ? preview
        .split("\n")
        .map((l) => l.trim())
        .find((l) => l && !l.startsWith("#") && !l.startsWith("|"))
        ?.slice(0, 100)
    : "";

  return (
    <li
      role="button"
      tabIndex={0}
      aria-pressed={active}
      className={`list-item ${active ? "selected" : ""}`}
      onClick={onSelect}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onSelect();
        }
      }}
    >
      <div className="list-item-title">{d.date}</div>
      {summary && <div className="list-item-preview">{summary}</div>}
      <div className="list-item-meta">
        {d.needs_attention > 0 && (
          <span className="pill warn">{d.needs_attention} attn</span>
        )}
        {!d.complete && <span className="pill warn">updating</span>}
        ign {d.ignorable} · flaky {d.flaky_candidates}
        {d.policy_gate && ` · ${d.policy_gate}`}
        {d.duration_label && ` · ${d.duration_label}`}
      </div>
    </li>
  );
}
