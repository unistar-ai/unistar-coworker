import type { ReactNode, ReactElement } from "react";
import { Children, cloneElement, isValidElement } from "react";

/** Split plain text into nodes with `<mark class="chat-search-hit">` around matches. */
export function highlightSearchText(text: string, query: string): ReactNode {
  const q = query.trim();
  if (!q || !text) return text;
  const lower = text.toLowerCase();
  const needle = q.toLowerCase();
  const parts: ReactNode[] = [];
  let start = 0;
  let idx = lower.indexOf(needle);
  let n = 0;
  while (idx >= 0) {
    if (idx > start) parts.push(text.slice(start, idx));
    parts.push(
      <mark key={`h-${n++}`} className="chat-search-hit">
        {text.slice(idx, idx + needle.length)}
      </mark>,
    );
    start = idx + needle.length;
    idx = lower.indexOf(needle, start);
  }
  if (start < text.length) parts.push(text.slice(start));
  return parts.length === 1 ? parts[0] : parts;
}

/** Recursively highlight string children inside a React node tree (for markdown). */
export function highlightReactNodes(node: ReactNode, query: string): ReactNode {
  const q = query.trim();
  if (!q) return node;
  return Children.map(node, (child) => {
    if (typeof child === "string" || typeof child === "number") {
      return highlightSearchText(String(child), q);
    }
    if (!isValidElement(child)) return child;
    const el = child as ReactElement<{ children?: ReactNode }>;
    if (el.type === "mark" || el.type === "code" || el.type === "pre") return child;
    if (el.props.children == null) return child;
    return cloneElement(el, {
      ...el.props,
      children: highlightReactNodes(el.props.children, q),
    });
  });
}
