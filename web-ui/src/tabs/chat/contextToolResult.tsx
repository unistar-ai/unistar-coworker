import { useMemo, useState, type MouseEvent } from "react";
import Markdown from "../../components/Markdown";
import { toolMeta } from "./parser";
import {
  formatToolOutputHtml,
  formatDiffHtml,
  looksLikeDiff,
  looksLikeJson,
  tryPrettyJson,
} from "./toolOutput";

export type ToolTranscriptKind = "result" | "error" | "approval" | "summarized" | "plain";

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

export function contextToolPreview(content: string, maxChars = 140): string {
  const p = parseContextToolTranscript(content);
  const name = p.toolName || "tool";
  const bodyOneLine = p.body.replace(/\s+/g, " ").trim();
  if (!bodyOneLine) {
    if (p.kind === "summarized") return `${name} (summarized)`;
    return name;
  }
  const lines = p.body.split("\n").length;
  if (lines > 1 && bodyOneLine.length > 48) {
    return `${name} · ${lines} lines`;
  }
  const snippet =
    bodyOneLine.length > maxChars - name.length - 3
      ? `${bodyOneLine.slice(0, maxChars - name.length - 4)}…`
      : bodyOneLine;
  return `${name} · ${snippet}`;
}

function bodyRendersAsMarkdown(toolName: string | null, body: string): boolean {
  const t = body.trimStart();
  if (!t) return false;
  if (toolName === "skill_load") return true;
  return /^#{1,3}\s/.test(t);
}

function ContextToolBody({ body }: { body: string }) {
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
    return <div className="ctx-tool-body-empty">_(no output body)_</div>;
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
              {expanded ? "Collapse" : `Show all (${effective.split("\n").length} lines)`}
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
              {prettyJson ? "Raw JSON" : "Pretty JSON"}
            </button>
          )}
          <button
            type="button"
            className={`ctx-tool-body-btn ctx-tool-body-copy${copied ? " is-copied" : ""}`}
            onClick={handleCopy}
          >
            {copied ? "Copied" : "Copy"}
          </button>
        </div>
      )}
    </div>
  );
}

const KIND_LABEL: Record<ToolTranscriptKind, string | null> = {
  result: null,
  error: "Error",
  approval: "Awaiting approval",
  summarized: "Summarized",
  plain: null,
};

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
  const useMarkdown = bodyRendersAsMarkdown(parsed.toolName, parsed.body);

  return (
    <div className={`ctx-tool-result kind-${parsed.kind}${parsed.ok ? "" : " is-error"}`}>
      {parsed.toolName && (
        <div className="ctx-tool-result-meta">
          <span className="ctx-tool-result-icon" aria-hidden="true">
            {meta?.icon ?? "⚙"}
          </span>
          <div className="ctx-tool-result-titles">
            <span className="ctx-tool-result-label">{meta?.label ?? parsed.toolName}</span>
            {meta && meta.label !== parsed.toolName && (
              <span className="ctx-tool-result-fn">{parsed.toolName}</span>
            )}
          </div>
          {kindLabel && (
            <span className={`ctx-tool-result-badge kind-${parsed.kind}`}>{kindLabel}</span>
          )}
          {meta?.source && (
            <span className="ctx-tool-result-source">{meta.source.source}</span>
          )}
        </div>
      )}

      {parsed.argsPretty && (
        <details className="ctx-tool-args">
          <summary className="ctx-tool-args-summary">Arguments</summary>
          <pre className="ctx-tool-args-pre">{parsed.argsPretty}</pre>
        </details>
      )}

      <div className="ctx-tool-body">
        {useMarkdown ? (
          <div className="ctx-tool-body-markdown prose-chat">
            <Markdown>{parsed.body}</Markdown>
          </div>
        ) : (
          <ContextToolBody body={parsed.body} />
        )}
      </div>
    </div>
  );
}
