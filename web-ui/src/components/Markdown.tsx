import { isValidElement } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import type { Components } from "react-markdown";
import CodeBlock from "./CodeBlock";

interface MarkdownProps {
  children: string;
  /** Streaming mode: appends a blinking cursor to the last block. */
  streaming?: boolean;
}

// Custom component mapping so code fences render with the legacy-style
// language badge + syntax highlighting (CodeBlock). Inline code stays as
// default <code> which is styled by .prose-chat code in index.css.
const components: Components = {
  code(props) {
    const { className, children, node, ...rest } = props;
    // react-markdown v9: inline code has no className (no language- prefix),
    // fenced code has `language-xxx`.
    const match = /language-(\w+)/.exec(className || "");
    const text = String(children);
    if (match) {
      return <CodeBlock code={text} lang={match[1]} />;
    }
    // Inline code — but react-markdown may also pass multi-line inline code
    // (rare). If it contains a newline, treat as a block.
    if (text.includes("\n")) {
      return <CodeBlock code={text} />;
    }
    return (
      <code className={className} {...rest}>
        {children}
      </code>
    );
  },
  // Open links in new tab + rel noopener (mirrors legacy safeLink behavior).
  a(props) {
    const { href, children } = props;
    const safe = isSafeUrl(href) ? href : "#";
    return (
      <a href={safe} target="_blank" rel="noopener noreferrer">
        {children}
      </a>
    );
  },
  // Ordered lists — add md-ol class so legacy counter-reset/::before styles apply.
  ol(props) {
    const { children, ...rest } = props;
    return (
      <ol className="md-ol" {...rest}>
        {children}
      </ol>
    );
  },
  // List items — detect GFM task list checkboxes and add the `task` class so
  // legacy flex/checkbox styling applies.
  li(props) {
    const { children, className, ...rest } = props;
    // react-markdown + remark-gfm renders task items with an <input
    // type="checkbox"> as the first child.
    const isTask = Array.isArray(children) && children.some(
      (c) =>
        isValidElement(c) &&
        c.type === "input" &&
        (c.props as { type?: string }).type === "checkbox",
    );
    return (
      <li className={isTask ? `task ${className || ""}`.trim() : className} {...rest}>
        {children}
      </li>
    );
  },
};

function isSafeUrl(href: string | undefined): boolean {
  if (!href) return false;
  const trimmed = href.trim();
  if (
    trimmed.startsWith("#") ||
    trimmed.startsWith("/") ||
    trimmed.startsWith("./") ||
    trimmed.startsWith("../")
  ) {
    return true;
  }
  const m = trimmed.match(/^([a-zA-Z][a-zA-Z0-9+.-]*):/);
  if (!m) return true; // relative
  const scheme = m[1].toLowerCase();
  return scheme === "http" || scheme === "https" || scheme === "mailto" || scheme === "tel";
}

export default function Markdown({ children, streaming }: MarkdownProps) {
  return (
    <div className={streaming ? "prose-chat is-streaming" : "prose-chat"}>
      <ReactMarkdown remarkPlugins={[remarkGfm]} components={components}>
        {children}
      </ReactMarkdown>
    </div>
  );
}
