import { isValidElement, useMemo, type ReactNode } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import type { Components } from "react-markdown";
import CodeBlock from "./CodeBlock";
import { highlightReactNodes } from "../tabs/chat/searchHighlight";

interface MarkdownProps {
  children: string;
  /** Streaming mode: appends a blinking cursor to the last block. */
  streaming?: boolean;
  /** Turn layout: larger type, calmer headings, tuned for chat history. */
  variant?: "default" | "turn";
  /** When set, wrap matching text in `<mark class="chat-search-hit">`. */
  highlightQuery?: string;
}

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

function wrapTextTag(
  Tag: "p" | "h1" | "h2" | "h3" | "h4" | "td" | "th" | "blockquote",
  highlightQuery: string | undefined,
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
): any {
  return ({ children, ...rest }: { children?: ReactNode }) => {
    const content =
      highlightQuery && highlightQuery.trim()
        ? highlightReactNodes(children, highlightQuery)
        : children;
    return <Tag {...rest}>{content}</Tag>;
  };
}

function buildComponents(highlightQuery?: string): Components {
  return {
    code(props) {
      const { className, children, node, ...rest } = props;
      const match = /language-(\w+)/.exec(className || "");
      const text = String(children);
      if (match) {
        return <CodeBlock code={text} lang={match[1]} />;
      }
      if (text.includes("\n")) {
        return <CodeBlock code={text} />;
      }
      return (
        <code className={className} {...rest}>
          {children}
        </code>
      );
    },
    a(props) {
      const { href, children } = props;
      const safe = isSafeUrl(href) ? href : "#";
      return (
        <a href={safe} target="_blank" rel="noopener noreferrer">
          {children}
        </a>
      );
    },
    img(props) {
      const { src, alt } = props;
      if (!src || !isSafeUrl(src)) return null;
      return (
        // eslint-disable-next-line jsx-a11y/no-noninteractive-element-interactions, jsx-a11y/click-events-have-key-events
        <img
          src={src}
          alt={alt || ""}
          loading="lazy"
          onClick={() => {
            try {
              window.open(src, "_blank", "noopener,noreferrer");
            } catch {
              /* popup blocked */
            }
          }}
        />
      );
    },
    ol(props) {
      const { children, ...rest } = props;
      return (
        <ol className="md-ol" {...rest}>
          {children}
        </ol>
      );
    },
    li(props) {
      const { children, className, ...rest } = props;
      const isTask =
        Array.isArray(children) &&
        children.some(
          (c) =>
            isValidElement(c) &&
            c.type === "input" &&
            (c.props as { type?: string }).type === "checkbox",
        );
      const content =
        highlightQuery && highlightQuery.trim()
          ? highlightReactNodes(children, highlightQuery)
          : children;
      return (
        <li className={isTask ? `task ${className || ""}`.trim() : className} {...rest}>
          {content}
        </li>
      );
    },
    table(props) {
      const { children, ...rest } = props;
      return (
        <div className="md-table-wrap">
          <table {...rest}>{children}</table>
        </div>
      );
    },
    p: wrapTextTag("p", highlightQuery),
    h1: wrapTextTag("h1", highlightQuery),
    h2: wrapTextTag("h2", highlightQuery),
    h3: wrapTextTag("h3", highlightQuery),
    h4: wrapTextTag("h4", highlightQuery),
    td: wrapTextTag("td", highlightQuery),
    th: wrapTextTag("th", highlightQuery),
    blockquote: wrapTextTag("blockquote", highlightQuery),
  };
}

export default function Markdown({
  children,
  streaming,
  variant = "default",
  highlightQuery,
}: MarkdownProps) {
  const cls = [
    "prose-chat",
    variant === "turn" ? "prose-turn" : "",
    streaming ? "is-streaming" : "",
  ]
    .filter(Boolean)
    .join(" ");
  const components = useMemo(() => buildComponents(highlightQuery), [highlightQuery]);
  return (
    <div className={cls}>
      <ReactMarkdown remarkPlugins={[remarkGfm]} components={components}>
        {children}
      </ReactMarkdown>
    </div>
  );
}
