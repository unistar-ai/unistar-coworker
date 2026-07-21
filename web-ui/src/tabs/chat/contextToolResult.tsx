import { useMemo, useState, type MouseEvent } from "react";
import { toolMeta } from "./parser";
import {
  argPairsFromRecord,
  preferArgBlock,
  prepareToolStepDisplay,
  toolArgSubtitle,
  toolRowTitle,
} from "./toolDisplay";
import { formatArgsShortFromRecord } from "./contextFocus";
import { ToolStepOutput } from "./toolOutput";
import ToolMarkdownToggle from "./ToolMarkdownToggle";
import {
  formatToolOutputHtml,
  formatDiffHtml,
  looksLikeDiff,
  looksLikeJson,
  tryPrettyJson,
} from "./toolOutput";

export type ToolTranscriptKind =
  | "result"
  | "error"
  | "approval"
  | "ask_user"
  | "summarized"
  | "plain";

export interface ParsedToolTranscript {
  kind: ToolTranscriptKind;
  toolName: string | null;
  ok: boolean;
  args: Record<string, unknown> | null;
  argsPretty: string | null;
  body: string;
}

function parseArgsAndBody(text: string): {
  argsPretty: string | null;
  args: Record<string, unknown> | null;
  body: string;
} {
  const trimmed = text.trimStart();
  if (!trimmed.startsWith("args:")) {
    return { argsPretty: null, args: null, body: text };
  }
  const afterMarker = trimmed.slice("args:".length);
  // Match Rust `parse_tool_transcript_args`: args block runs until a blank line.
  const bodySep = afterMarker.search(/\n\s*\n/);
  const argsBlock = (bodySep === -1 ? afterMarker : afterMarker.slice(0, bodySep)).trim();
  let body =
    bodySep === -1
      ? ""
      : afterMarker
          .slice(bodySep)
          .replace(/^\s*\n+/, "")
          .replace(/^\s*:\s*/, "");
  try {
    const parsed = JSON.parse(argsBlock) as Record<string, unknown>;
    return {
      argsPretty: JSON.stringify(parsed, null, 2),
      args: parsed,
      body,
    };
  } catch {
    return { argsPretty: argsBlock || null, args: null, body };
  }
}

/** Parse `tool_result` / `tool_error` transcripts from the LLM context panel. */
export function parseContextToolTranscript(content: string): ParsedToolTranscript {
  const trimmed = content.trimStart();

  const summarized = trimmed.match(/^\[summarized tool_result ([^\]]+)\]\s*/);
  if (summarized) {
    return {
      kind: "summarized",
      toolName: summarized[1].trim(),
      ok: true,
      args: null,
      argsPretty: null,
      body: trimmed.slice(summarized[0].length),
    };
  }

  const prefixes: { prefix: string; kind: ToolTranscriptKind; ok: boolean }[] = [
    { prefix: "tool_result(", kind: "result", ok: true },
    { prefix: "tool_error(", kind: "error", ok: false },
    { prefix: "tool_approval_pending(", kind: "approval", ok: false },
    { prefix: "tool_user_question_pending(", kind: "ask_user", ok: true },
  ];

  for (const { prefix, kind, ok } of prefixes) {
    if (!trimmed.startsWith(prefix)) continue;
    const rest = trimmed.slice(prefix.length);
    const close = rest.indexOf("):");
    if (close === -1) break;
    const namePart = rest.slice(0, close);
    const toolName = namePart.split(",")[0]?.trim() || null;
    const afterHeader = rest.slice(close + 2).replace(/^\s*:\s*/, "");
    const { args, argsPretty, body } = parseArgsAndBody(afterHeader);
    return { kind, toolName, ok, args, argsPretty, body };
  }

  return {
    kind: "plain",
    toolName: null,
    ok: true,
    args: null,
    argsPretty: null,
    body: content,
  };
}

function argRecordToShortLine(args: Record<string, unknown> | null): string | null {
  if (!args) return null;
  const pairs = argPairsFromRecord(args);
  if (!pairs.length) return null;
  // Reconstruct a parseable key=value line for toolArgSubtitle.
  return pairs.map((p) => `${p.key}=${p.value}`).join(", ");
}

/** Collapsed preview for a context tool message. */
export function contextToolPreview(
  content: string,
  maxChars = 140,
  mcpPrefixes: { id: string; prefix: string }[] = [],
): string {
  const p = parseContextToolTranscript(content);
  const meta = p.toolName ? toolMeta(p.toolName, mcpPrefixes) : null;
  const title = p.toolName
    ? toolRowTitle(p.toolName, meta?.label ?? p.toolName, p.ok ? "ok" : "err")
    : "tool";

  const argsLine = argRecordToShortLine(p.args);
  const argSub = p.toolName ? toolArgSubtitle(p.toolName, argsLine) : null;
  const prepared = prepareToolStepDisplay(p.toolName, content);
  const bodyLine = prepared.body.replace(/\s+/g, " ").trim();

  let detail = argSub || formatArgsShortFromRecord(p.args) || null;
  if (!detail && bodyLine) {
    detail =
      bodyLine.length > 72 ? `${bodyLine.slice(0, 71)}…` : bodyLine;
  }
  if (p.kind === "summarized" && !detail) detail = "已摘要";
  if (p.kind === "error" && !detail) detail = "失败";
  if (p.kind === "approval") detail = detail || "等待批准";
  if (p.kind === "ask_user") detail = detail || "等待回答";

  const line = detail ? `${title} · ${detail}` : title;
  if (line.length <= maxChars) return line;
  return `${line.slice(0, maxChars - 1)}…`;
}

export function bodyRendersAsMarkdown(toolName: string | null, body: string): boolean {
  return prepareToolStepDisplay(toolName, body).display === "markdown";
}

function ContextToolBodyFallback({ body }: { body: string }) {
  const [expanded, setExpanded] = useState(true);
  const [prettyJson, setPrettyJson] = useState(true);
  const [copied, setCopied] = useState(false);

  const isDiff = looksLikeDiff(body);
  const isJson = !isDiff && looksLikeJson(body);
  const collapsible = body.split("\n").length > 8 || body.length > 600;
  const effective = isJson && prettyJson ? tryPrettyJson(body) : body;
  const displayText =
    collapsible && !expanded
      ? effective.split("\n").slice(0, 6).join("\n") + "\n…"
      : effective;
  const html = isDiff ? formatDiffHtml(displayText) : formatToolOutputHtml(displayText);

  const handleCopy = async (e: MouseEvent) => {
    e.stopPropagation();
    try {
      await navigator.clipboard.writeText(effective);
      setCopied(true);
      setTimeout(() => setCopied(false), 1200);
    } catch {
      // clipboard unavailable
    }
  };

  if (!body.trim()) {
    return <div className="ctx-tool-body-empty">无输出</div>;
  }

  return (
    <div
      className={`ctx-tool-body-panel${isDiff ? " is-diff" : ""}${isJson ? " is-json" : ""}${
        expanded ? " is-expanded" : ""
      }`}
    >
      <pre className="ctx-tool-body-pre" dangerouslySetInnerHTML={{ __html: html }} />
      {(collapsible || isJson) && (
        <div className="ctx-tool-body-actions">
          {collapsible && (
            <button
              type="button"
              className="ctx-tool-body-btn"
              onClick={(e) => {
                e.stopPropagation();
                setExpanded((v) => !v);
              }}
            >
              {expanded ? "收起" : `展开全部（${effective.split("\n").length} 行）`}
            </button>
          )}
          {isJson && (
            <button
              type="button"
              className="ctx-tool-body-btn"
              onClick={(e) => {
                e.stopPropagation();
                setPrettyJson((v) => !v);
              }}
            >
              {prettyJson ? "原始 JSON" : "格式化 JSON"}
            </button>
          )}
          <button
            type="button"
            className={`ctx-tool-body-btn ctx-tool-body-copy${copied ? " is-copied" : ""}`}
            onClick={handleCopy}
          >
            {copied ? "已复制" : "复制"}
          </button>
        </div>
      )}
    </div>
  );
}

const KIND_LABEL: Record<ToolTranscriptKind, string | null> = {
  result: null,
  error: "错误",
  approval: "等待批准",
  ask_user: "等待回答",
  summarized: "已摘要",
  plain: null,
};

function ContextToolArgsPane({
  args,
  argsPretty,
}: {
  args: Record<string, unknown> | null;
  argsPretty: string | null;
}) {
  const pairs = useMemo(() => argPairsFromRecord(args), [args]);
  if (pairs.length === 0 && !argsPretty) return null;

  const blockArgs = pairs.filter((p) => preferArgBlock(p.key, p.value));
  const listArgs = pairs.filter((p) => !preferArgBlock(p.key, p.value));

  return (
    <section className="tool-detail-pane is-args" aria-label="参数">
      <header className="tool-detail-pane-head">
        <span className="tool-detail-pane-label">参数</span>
      </header>
      <div className="tool-detail-pane-body">
        {pairs.length > 0 ? (
          <>
            {blockArgs.map((p) => (
              <div key={p.key} className="tool-inline-command">
                <div className="tool-inline-command-label">{p.key}</div>
                <pre className="tool-inline-command-body">{p.value}</pre>
              </div>
            ))}
            {listArgs.length > 0 && (
              <dl className="tool-arg-list">
                {listArgs.map((p, i) => (
                  <div key={`${p.key}-${i}`} className="tool-arg-list-row">
                    <dt className="tool-arg-list-k">{p.key}</dt>
                    <dd className="tool-arg-list-v">{p.value || "—"}</dd>
                  </div>
                ))}
              </dl>
            )}
          </>
        ) : (
          <pre className="tool-inline-command-body">{argsPretty}</pre>
        )}
      </div>
    </section>
  );
}

export function ContextToolResultView({
  content,
  mcpPrefixes,
}: {
  content: string;
  mcpPrefixes: { id: string; prefix: string }[];
}) {
  const parsed = useMemo(() => parseContextToolTranscript(content), [content]);
  const meta = parsed.toolName ? toolMeta(parsed.toolName, mcpPrefixes) : null;
  const kindLabel = KIND_LABEL[parsed.kind];
  const hasArgs = Boolean(parsed.args && Object.keys(parsed.args).length) || Boolean(parsed.argsPretty);
  const hasBody = Boolean(parsed.body.trim());

  return (
    <div className={`ctx-tool-result kind-${parsed.kind}${parsed.ok ? "" : " is-error"}`}>
      {(kindLabel || meta?.source) && (
        <div className="ctx-tool-result-meta is-compact">
          {kindLabel && (
            <span className={`ctx-tool-result-badge kind-${parsed.kind}`}>{kindLabel}</span>
          )}
          {meta?.source && (
            <span className="ctx-tool-result-source">{meta.source.source}</span>
          )}
          {parsed.toolName && (
            <span className="ctx-tool-result-fn">{parsed.toolName}</span>
          )}
        </div>
      )}

      <div className="ctx-tool-panes">
        {hasArgs && (
          <ContextToolArgsPane args={parsed.args} argsPretty={parsed.argsPretty} />
        )}

        <section className="tool-detail-pane is-result" aria-label="结果">
          <header className="tool-detail-pane-head">
            <span className="tool-detail-pane-label">结果</span>
            <ToolMarkdownToggle className="tool-detail-pane-action" />
          </header>
          <div className="tool-detail-pane-body">
            {hasBody ? (
              parsed.kind === "plain" && !parsed.toolName ? (
                <ContextToolBodyFallback body={parsed.body} />
              ) : (
                <ToolStepOutput
                  output={content}
                  outputKey="ctx-tool"
                  toolName={parsed.toolName ?? undefined}
                  inline
                />
              )
            ) : (
              <div className="ctx-tool-body-empty">无输出</div>
            )}
          </div>
        </section>
      </div>
    </div>
  );
}
