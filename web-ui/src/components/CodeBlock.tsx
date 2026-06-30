import { useEffect, useMemo, useState } from "react";
import { Check, Copy } from "lucide-react";
import { resolveLang } from "../lib/lang";

// Regex fallback highlighter — ported from legacy markdown.js::highlightCode.
// Used for the first paint (instant) and for languages highlight.js doesn't load.
// Operates on already-escaped HTML text.

function escapeHtml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function highlightCodeRegex(codeEscaped: string, lang: string): string {
  const L = (lang || "").toLowerCase();
  const str = (s: string) => `<span class="tok-string">${s}</span>`;
  const kw = (s: string) => `<span class="tok-kw">${s}</span>`;
  const cm = (s: string) => `<span class="tok-comment">${s}</span>`;
  const ky = (s: string) => `<span class="tok-key">${s}</span>`;

  if (L === "bash" || L === "sh" || L === "shell" || L === "zsh") {
    return codeEscaped
      .replace(/(^|\n)(\s*#.*)/g, (_m, prefix: string, comment: string) => `${prefix}${cm(comment)}`)
      .replace(/(&quot;[^&]*&quot;|'[^']*')/g, (m) => str(m))
      .replace(
        /\b(if|then|else|elif|fi|for|do|done|echo|cd|exit|export|source|sudo|curl|wget|grep)\b/g,
        (m) => kw(m),
      );
  }
  if (L === "json") {
    return codeEscaped
      .replace(/(&quot;[^&]*&quot;)(\s*:)/g, (_m, k: string, colon: string) => `${ky(k)}${colon}`)
      .replace(/:\s*(&quot;[^&]*&quot;)/g, (_m, v: string) => `: ${str(v)}`)
      .replace(/\b(true|false|null)\b/g, (m) => kw(m));
  }
  if (L === "rust" || L === "rs") {
    const kws =
      "fn|let|mut|pub|use|struct|enum|impl|match|if|else|return|async|await|true|false|Some|None|Ok|Err";
    return codeEscaped
      .replace(/(\/\/.*)/g, (m) => cm(m))
      .replace(/(&quot;[^&]*&quot;)/g, (m) => str(m))
      .replace(new RegExp(`\\b(${kws})\\b`, "g"), (m) => kw(m));
  }
  if (L === "javascript" || L === "js" || L === "typescript" || L === "ts") {
    const kws =
      "function|const|let|var|return|if|else|async|await|import|export|from|true|false|null|undefined|class|new";
    return codeEscaped
      .replace(/(\/\/.*)/g, (m) => cm(m))
      .replace(/(&quot;[^&]*&quot;|`[^`]*`|'[^']*')/g, (m) => str(m))
      .replace(new RegExp(`\\b(${kws})\\b`, "g"), (m) => kw(m));
  }
  return codeEscaped;
}

interface CodeBlockProps {
  code: string;
  lang?: string;
}

export default function CodeBlock({ code, lang }: CodeBlockProps) {
  const hljsLang = resolveLang(lang);

  // First paint: regex highlight (or plain escaped text for unsupported langs).
  const fallbackHtml = useMemo(() => {
    const escaped = escapeHtml(code.replace(/\n$/, ""));
    return highlightCodeRegex(escaped, lang || "");
  }, [code, lang]);

  // `html` holds the inner `<code>` content (either regex fallback or
  // highlight.js `<span class="hljs-…">` tokens). Always injected into
  // `<pre><code>`, so there is no inline-style/CSP concern.
  const [html, setHtml] = useState(fallbackHtml);
  const [copied, setCopied] = useState(false);

  // When code/lang changes, kick off highlight.js and swap in the result.
  // The highlight module is dynamically imported so the heavy hljs core +
  // language grammars land in a separate chunk, fetched only on first highlight.
  useEffect(() => {
    let cancelled = false;
    if (!hljsLang) {
      setHtml(fallbackHtml);
      return;
    }
    void import("../lib/highlight")
      .then(({ highlightAsync }) => highlightAsync(code.replace(/\n$/, ""), hljsLang))
      .then((hljsHtml) => {
        if (!cancelled) setHtml(hljsHtml);
      })
      .catch(() => {
        if (!cancelled) setHtml(fallbackHtml);
      });
    return () => {
      cancelled = true;
    };
  }, [code, lang, hljsLang, fallbackHtml]);

  // Reset to fallback immediately when the input changes so we never show
  // stale highlighted HTML for the previous content while hljs re-runs.
  useEffect(() => {
    setHtml(fallbackHtml);
  }, [fallbackHtml]);

  const langLabel = lang && lang.length > 0 ? lang : "";

  const onCopy = async () => {
    try {
      await navigator.clipboard.writeText(code);
      setCopied(true);
      setTimeout(() => setCopied(false), 1200);
    } catch {
      /* clipboard unavailable */
    }
  };

  return (
    <div className="md-code-block">
      {langLabel && <span className="md-code-lang">{langLabel}</span>}
      <button
        type="button"
        className="md-code-copy"
        onClick={onCopy}
        aria-label="Copy code"
        title="Copy code"
      >
        {copied ? <Check size={13} /> : <Copy size={13} />}
      </button>
      <pre>
        <code dangerouslySetInnerHTML={{ __html: html }} />
      </pre>
    </div>
  );
}
