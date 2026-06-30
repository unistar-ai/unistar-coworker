import { useStore } from "../../store/wsStore";
import { apiPost } from "../../lib/api";
import Markdown from "../../components/Markdown";
import EmptyState from "../../components/EmptyState";
import Skeleton from "../../components/Skeleton";
import { GitPullRequest, Inbox } from "lucide-react";
import type { PrSnapshot } from "../../store/protocol";

export default function PrsTab() {
  const prs = useStore((s) => s.prs);
  const selectedIdx = useStore((s) => s.selected_pr_index);
  const prFilter = useStore((s) => s.pr_filter);
  const prSort = useStore((s) => s.pr_sort);
  const overview = useStore((s) => s.pr_overview);
  const overviewLoading = useStore((s) => s.pr_overview_loading);

  return (
    <div className="split-panel">
      <div className="split-list">
        <div className="toolbar">
          <button
            type="button"
            className="btn btn-ghost"
            onClick={() => void apiPost("/api/prs/filter")}
          >
            Filter: {prFilter || "all"}
          </button>
          <button
            type="button"
            className="btn btn-ghost"
            onClick={() => void apiPost("/api/prs/sort")}
          >
            Sort: {prSort || "default"}
          </button>
          <button
            type="button"
            className="btn btn-ghost"
            disabled={!prs.length}
            onClick={() => void apiPost(`/api/prs/${selectedIdx}/triage`)}
          >
            Triage
          </button>
        </div>
        {prs.length === 0 ? (
          <EmptyState
            icon={GitPullRequest}
            title="No PRs in store"
            description="Pull requests will appear here once the review-radar workflow populates the store."
            action={
              <button
                type="button"
                className="btn btn-primary"
                onClick={() => void apiPost("/api/workflows/review-radar")}
              >
                Run review-radar
              </button>
            }
          />
        ) : (
          <ul className="list">
            {prs.map((p, i) => (
              <PrRow
                key={`${p.repo}-${p.number}`}
                p={p}
                active={i === selectedIdx}
                onSelect={async () => {
                  await apiPost(`/api/prs/${i}/select`);
                  await apiPost(`/api/prs/${i}/overview`);
                }}
              />
            ))}
          </ul>
        )}
      </div>
      <div className="split-detail">
        {overviewLoading ? (
          <Skeleton rows={6} className="overview-skeleton" />
        ) : overview ? (
          <Markdown>{overview}</Markdown>
        ) : prs.length ? (
          <EmptyState
            icon={Inbox}
            title="Select a PR"
            description="Pick a pull request on the left to load its overview."
          />
        ) : null}
      </div>
    </div>
  );
}

function ciPill(summary: string): { label: string; cls: string } {
  const s = (summary || "").toLowerCase();
  if (s.startsWith("failing")) return { label: summary, cls: "err" };
  if (s.startsWith("pending")) return { label: summary, cls: "warn" };
  if (s.startsWith("passing")) return { label: summary, cls: "ok" };
  return { label: summary || "none", cls: "muted" };
}

function PrRow({
  p,
  active,
  onSelect,
}: {
  p: PrSnapshot;
  active: boolean;
  onSelect: () => Promise<void>;
}) {
  const pill = ciPill(p.ci_summary);
  return (
    <li
      role="button"
      tabIndex={0}
      aria-pressed={active}
      className={`list-item ${active ? "selected" : ""}`}
      onClick={() => void onSelect()}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          void onSelect();
        }
      }}
    >
      <div className="list-item-title">
        <span className={`pill ${pill.cls}`}>{pill.label}</span>
        #{p.number} {p.title}
        {p.triage_note && <span className="triage-mark" title="triage"> ◆</span>}
      </div>
      <div className="list-item-meta">
        {p.repo} · {p.author}
        {p.is_draft && " · draft"}
        {p.review_summary && ` · ${p.review_summary}`}
      </div>
    </li>
  );
}
