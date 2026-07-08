import { useState, type MouseEvent } from "react";

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
}: {
  output: string;
  outputKey: string;
}) {
  const [expanded, setExpanded] = useState(false);
  const [pretty, setPretty] = useState(true);
  const [copied, setCopied] = useState(false);
  const lines = output.split("\n");
  const collapsible = lines.length > 6 || output.length > 480;
  const isDiff = looksLikeDiff(output);
  const isJson = !isDiff && looksLikeJson(output) && (output.length > 480 || lines.length > 6);
  const isBash = looksLikeBashOutput(output);

  // For long JSON, allow toggling between pretty-printed and raw.
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

  return (
    <div
      key={outputKey}
      className={`tool-output-wrap${expanded && collapsible ? " is-expanded" : ""}${isDiff ? " is-diff" : ""}${isJson ? " is-json" : ""}`}
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
            {expanded ? "Collapse output" : `Show all ${effLines.length} lines`}
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
              {pretty ? "Raw" : "Pretty"}
            </button>
          )}
          {expanded && (
            <button
              type="button"
              className={`tool-output-copy${copied ? " is-copied" : ""}`}
              onClick={handleCopy}
            >
              {copied ? "Copied ✓" : "Copy"}
            </button>
          )}
        </div>
      )}
    </div>
  );
}
