import { useMemo, useState, type MouseEvent } from "react";
import Markdown from "../../components/Markdown";
import { useChatUiStore } from "../../store/chatUiStore";
import {
  parseAskUserBody,
  prepareToolStepDisplay,
  parseShellTranscript,
  transcriptKindLabel,
} from "./toolDisplay";

export function escapeHtml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

/** Highlight common bash/tool output patterns. */
export function formatToolOutputHtml(text: string): string {
  const lines = text.split("\n");
  return lines
    .map((line) => {
      const esc = escapeHtml(line);
      if (/^exit:\s*\d+/i.test(line)) {
        const ok = /exit:\s*0\b/i.test(line);
        return `<span class="out-line out-exit ${ok ? "ok" : "err"}">${esc}</span>`;
      }
      if (/^stderr:/i.test(line)) return `<span class="out-line out-stderr">${esc}</span>`;
      if (/^stdout:/i.test(line)) return `<span class="out-line out-stdout">${esc}</span>`;
      if (/^cwd:/i.test(line)) return `<span class="out-line out-meta">${esc}</span>`;
      if (/error|failed|invalid/i.test(line)) return `<span class="out-line out-err">${esc}</span>`;
      return `<span class="out-line">${esc}</span>`;
    })
    .join("\n");
}

/** Detect unified-diff output: `diff --git` or `@@ ... @@` hunk markers. */
export function looksLikeDiff(text: string): boolean {
  return /^diff --git\b/m.test(text) || /^@@.*@@/m.test(text);
}

/** Detect whether the output is a JSON document (object or array). */
export function looksLikeJson(text: string): boolean {
  const t = text.trim();
  if (!t) return false;
  if (!(t.startsWith("{") || t.startsWith("["))) return false;
  try {
    JSON.parse(t);
    return true;
  } catch {
    return false;
  }
}

/** Pretty-print JSON with 2-space indent; returns the original on failure. */
export function tryPrettyJson(text: string): string {
  try {
    return JSON.stringify(JSON.parse(text.trim()), null, 2);
  } catch {
    return text;
  }
}

/** Render a unified diff with +/- line tinting and left gutter markers. */
export function formatDiffHtml(text: string): string {
  const lines = text.split("\n");
  return lines
    .map((line) => {
      const esc = escapeHtml(line);
      if (/^diff --git\b/.test(line)) return `<span class="diff-line diff-meta">${esc}</span>`;
      if (/^index /.test(line)) return `<span class="diff-line diff-meta">${esc}</span>`;
      if (/^--- /.test(line)) return `<span class="diff-line diff-header diff-del">${esc}</span>`;
      if (/^\+\+\+ /.test(line)) return `<span class="diff-line diff-header diff-add">${esc}</span>`;
      if (/^@@.*@@/.test(line)) return `<span class="diff-line diff-hunk">${esc}</span>`;
      if (/^\+/.test(line)) return `<span class="diff-line diff-add"><span class="diff-gutter">+</span>${esc.slice(1)}</span>`;
      if (/^-/.test(line)) return `<span class="diff-line diff-del"><span class="diff-gutter">-</span>${esc.slice(1)}</span>`;
      if (/^\\ No newline/.test(line)) return `<span class="diff-line diff-meta">${esc}</span>`;
      return `<span class="diff-line diff-ctx"><span class="diff-gutter"> </span>${esc}</span>`;
    })
    .join("\n");
}

/** Bash / shell tool transcripts: exit line, stdout/stderr prefixes. */
export function looksLikeBashOutput(text: string): boolean {
  return /^exit:\s*\d+/m.test(text) || /^stdout:/m.test(text) || /^stderr:/m.test(text);
}

/** Collapsed view: keep head + tail so long build logs still show the exit line. */
export function collapseLongOutput(
  text: string,
  headLines = 4,
  tailLines = 3,
): string {
  const lines = text.split("\n");
  if (lines.length <= headLines + tailLines + 1) {
    return text;
  }
  const omitted = lines.length - headLines - tailLines;
  return [
    ...lines.slice(0, headLines),
    `… (${omitted} lines omitted) …`,
    ...lines.slice(-tailLines),
  ].join("\n");
}

export function ToolOutputView({
  output,
  outputKey,
  isError = false,
  inline = false,
}: {
  output: string;
  outputKey: string;
  isError?: boolean;
  inline?: boolean;
}) {
  const [expanded, setExpanded] = useState(false);
  const [pretty, setPretty] = useState(true);
  const [copied, setCopied] = useState(false);
  const lines = output.split("\n");
  const collapsible = lines.length > 6 || output.length > 480;
  const isDiff = looksLikeDiff(output);
  const isJson = !isDiff && looksLikeJson(output) && (output.length > 480 || lines.length > 6);
  const isBash = looksLikeBashOutput(output);

  const effectiveOutput = isJson && pretty ? tryPrettyJson(output) : output;
  const effLines = effectiveOutput.split("\n");
  const displayText = (() => {
    if (!collapsible || expanded) {
      return effectiveOutput;
    }
    if (isBash && effLines.length > 8) {
      return collapseLongOutput(effectiveOutput);
    }
    return effLines.slice(0, 5).join("\n") + "\n…";
  })();

  const html = isDiff ? formatDiffHtml(displayText) : formatToolOutputHtml(displayText);

  const handleCopy = async (e: MouseEvent) => {
    e.stopPropagation();
    e.preventDefault();
    try {
      await navigator.clipboard.writeText(effectiveOutput);
      setCopied(true);
      setTimeout(() => setCopied(false), 1200);
    } catch {
      // clipboard may be unavailable
    }
  };

  if (!output.trim()) {
    return <div className="tool-output-empty">无输出</div>;
  }

  return (
    <div
      key={outputKey}
      className={`tool-output-wrap${expanded && collapsible ? " is-expanded" : ""}${isDiff ? " is-diff" : ""}${isJson ? " is-json" : ""}${isError ? " is-error" : ""}${inline ? " is-inline" : ""}`}
    >
      <pre
        className="tool-output"
        dangerouslySetInnerHTML={{ __html: html }}
      />
      {collapsible && (
        <div className="tool-output-actions">
          <button
            type="button"
            className="tool-output-toggle"
            onClick={(e) => {
              e.stopPropagation();
              e.preventDefault();
              setExpanded((v) => !v);
            }}
          >
            {expanded ? "收起" : `展开全部 ${effLines.length} 行`}
          </button>
          {isJson && (
            <button
              type="button"
              className="tool-output-toggle"
              onClick={(e) => {
                e.stopPropagation();
                e.preventDefault();
                setPretty((v) => !v);
              }}
            >
              {pretty ? "原始 JSON" : "格式化"}
            </button>
          )}
          {expanded && (
            <button
              type="button"
              className={`tool-output-copy${copied ? " is-copied" : ""}`}
              onClick={handleCopy}
            >
              {copied ? "已复制" : "复制"}
            </button>
          )}
        </div>
      )}
    </div>
  );
}

function ShellOutputView({
  body,
  outputKey,
  inline,
  isError,
}: {
  body: string;
  outputKey: string;
  inline?: boolean;
  isError?: boolean;
}) {
  const parts = useMemo(() => parseShellTranscript(body), [body]);
  if (!parts) {
    return (
      <ToolOutputView
        output={body}
        outputKey={outputKey}
        isError={isError}
        inline={inline}
      />
    );
  }

  const exitOk = parts.exit ? /^0\b/.test(parts.exit) : !isError;

  return (
    <div
      className={`tool-shell-output${inline ? " is-inline" : ""}${isError ? " is-error" : ""}`}
      key={outputKey}
    >
      {(parts.exit || parts.cwd) && (
        <div className="tool-shell-meta">
          {parts.exit && (
            <span className={`tool-shell-chip${exitOk ? " is-ok" : " is-err"}`}>
              exit {parts.exit}
            </span>
          )}
          {parts.cwd && (
            <span className="tool-shell-chip is-cwd" title={parts.cwd}>
              {parts.cwd}
            </span>
          )}
        </div>
      )}
      {parts.stdout?.trim() ? (
        <ToolOutputView
          output={parts.stdout}
          outputKey={`${outputKey}-stdout`}
          inline={inline}
        />
      ) : null}
      {parts.stderr?.trim() ? (
        <div className="tool-shell-stderr">
          <div className="tool-shell-section-label">stderr</div>
          <ToolOutputView
            output={parts.stderr}
            outputKey={`${outputKey}-stderr`}
            isError
            inline={inline}
          />
        </div>
      ) : null}
      {!parts.stdout?.trim() && !parts.stderr?.trim() && (
        <div className="tool-output-empty">(无输出)</div>
      )}
    </div>
  );
}

function AskUserOutputView({
  body,
  outputKey,
  inline,
}: {
  body: string;
  outputKey: string;
  inline?: boolean;
}) {
  const parsed = useMemo(() => parseAskUserBody(body), [body]);
  return (
    <div className={`tool-ask-user-output${inline ? " is-inline" : ""}`} key={outputKey}>
      {parsed.question && (
        <div className="tool-ask-user-row">
          <span className="tool-ask-user-label">问题</span>
          <span className="tool-ask-user-value">{parsed.question}</span>
        </div>
      )}
      {parsed.options.length > 0 && (
        <ul className="tool-ask-user-options">
          {parsed.options.map((opt, i) => (
            <li key={i}>{opt}</li>
          ))}
        </ul>
      )}
      {parsed.answer && (
        <div className="tool-ask-user-row is-answer">
          <span className="tool-ask-user-label">回答</span>
          <span className="tool-ask-user-value">{parsed.answer}</span>
        </div>
      )}
      {parsed.pending && !parsed.answer && (
        <div className="tool-ask-user-pending">等待你的回答…</div>
      )}
    </div>
  );
}

function SummarizedOutputView({
  body,
  outputKey,
  inline,
}: {
  body: string;
  outputKey: string;
  inline?: boolean;
}) {
  return (
    <div className={`tool-summarized-output${inline ? " is-inline" : ""}`} key={outputKey}>
      <span className="tool-summarized-badge">已摘要</span>
      <ToolOutputView output={body} outputKey={`${outputKey}-body`} inline={inline} />
    </div>
  );
}

function ReadFileOutputView({
  body,
  outputKey,
  inline,
}: {
  body: string;
  outputKey: string;
  inline?: boolean;
}) {
  const lines = body.split("\n");
  const numbered = lines.some((l) => /^\d+\|/.test(l));
  if (!numbered) {
    return <ToolOutputView output={body} outputKey={outputKey} inline={inline} />;
  }
  return (
    <div
      key={outputKey}
      className={`tool-read-file-output${inline ? " is-inline" : ""}`}
    >
      {lines.map((line, i) => {
        const m = line.match(/^(\d+)\|(.*)$/);
        if (m) {
          return (
            <div key={i} className="tool-read-file-line">
              <span className="tool-read-file-ln">{m[1]}</span>
              <span className="tool-read-file-code">{m[2] || " "}</span>
            </div>
          );
        }
        if (!line.trim()) return <div key={i} className="tool-read-file-gap" />;
        return (
          <div key={i} className="tool-read-file-plain">
            {line}
          </div>
        );
      })}
    </div>
  );
}

function GrepOutputView({
  body,
  outputKey,
  inline,
}: {
  body: string;
  outputKey: string;
  inline?: boolean;
}) {
  const lines = body.split("\n").filter((l) => l.trim());
  if (lines.length === 0 || /no matches/i.test(body)) {
    return (
      <div className={`tool-grep-empty${inline ? " is-inline" : ""}`} key={outputKey}>
        {body.trim() || "无匹配"}
      </div>
    );
  }
  return (
    <div key={outputKey} className={`tool-grep-output${inline ? " is-inline" : ""}`}>
      {lines.map((line, i) => (
        <div key={i} className="tool-grep-line">
          {line}
        </div>
      ))}
    </div>
  );
}

/** Strip LLM transcript envelope and render tool body with type-aware layout. */
export function ToolStepOutput({
  output,
  outputKey,
  toolName,
  inline = false,
  preferMarkdown,
}: {
  output: string;
  outputKey: string;
  toolName?: string;
  inline?: boolean;
  /** Override store preference (mainly for tests). */
  preferMarkdown?: boolean;
}) {
  const storeMarkdown = useChatUiStore((s) => s.toolMarkdown);
  const toolMarkdown = preferMarkdown ?? storeMarkdown;
  const prepared = useMemo(
    () => prepareToolStepDisplay(toolName, output),
    [toolName, output],
  );
  const kindLabel = transcriptKindLabel(prepared.parsed.kind);

  if (prepared.display === "ask_user") {
    return <AskUserOutputView body={prepared.body} outputKey={outputKey} inline={inline} />;
  }

  if (prepared.display === "shell") {
    return (
      <ShellOutputView
        body={prepared.body}
        outputKey={outputKey}
        inline={inline}
        isError={prepared.error}
      />
    );
  }

  if (prepared.display === "summarized") {
    return (
      <SummarizedOutputView body={prepared.body} outputKey={outputKey} inline={inline} />
    );
  }

  if (prepared.display === "read_file") {
    return (
      <ReadFileOutputView body={prepared.body} outputKey={outputKey} inline={inline} />
    );
  }

  if (prepared.display === "grep") {
    return (
      <GrepOutputView body={prepared.body} outputKey={outputKey} inline={inline} />
    );
  }

  if (prepared.display === "markdown" && prepared.body.trim() && toolMarkdown) {
    return (
      <div
        key={outputKey}
        className={`tool-output-markdown prose-chat${inline ? " is-inline" : ""}${prepared.error ? " is-error" : ""}`}
      >
        {kindLabel && (
          <span className={`tool-output-kind-badge kind-${prepared.parsed.kind}`}>
            {kindLabel}
          </span>
        )}
        <Markdown variant={inline ? "turn" : undefined}>{prepared.body}</Markdown>
      </div>
    );
  }

  return (
    <div className={prepared.error ? "tool-output-error-wrap" : undefined}>
      {kindLabel && (
        <span className={`tool-output-kind-badge kind-${prepared.parsed.kind}`}>
          {kindLabel}
        </span>
      )}
      <ToolOutputView
        output={prepared.body}
        outputKey={outputKey}
        isError={prepared.error}
        inline={inline}
      />
    </div>
  );
}
