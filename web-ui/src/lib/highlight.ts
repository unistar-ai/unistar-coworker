// highlight.js-based async syntax highlighting. This module is lazy-loaded by
// CodeBlock via a dynamic import() so none of it lands in the main entry chunk.
// A single hljs core is created lazily and reused; per-language results are
// cached so repeated renders are instant.
//
// Unlike shiki, highlight.js emits CSS classes (hljs-keyword, hljs-string, …)
// rather than inline style attributes, so it is fully compatible with a strict
// CSP of `style-src 'self'` (no 'unsafe-inline' needed). Colors come from the
// .hljs-* rules in index.css, which reference our --tok-* theme variables.

import hljs from "highlight.js/lib/core";
import type { HljsLang } from "./lang";

// Re-export so existing imports from "../lib/highlight" keep working.
export type { HljsLang } from "./lang";

// Dynamic imports — Vite code-splits each language into its own chunk,
// loaded only when first needed.
const LANG_LOADERS: Record<HljsLang, () => Promise<unknown>> = {
  bash: () => import("highlight.js/lib/languages/bash"),
  shell: () => import("highlight.js/lib/languages/shell"),
  json: () => import("highlight.js/lib/languages/json"),
  rust: () => import("highlight.js/lib/languages/rust"),
  javascript: () => import("highlight.js/lib/languages/javascript"),
  typescript: () => import("highlight.js/lib/languages/typescript"),
  python: () => import("highlight.js/lib/languages/python"),
  go: () => import("highlight.js/lib/languages/go"),
  yaml: () => import("highlight.js/lib/languages/yaml"),
  sql: () => import("highlight.js/lib/languages/sql"),
  ini: () => import("highlight.js/lib/languages/ini"),
  diff: () => import("highlight.js/lib/languages/diff"),
};

const loadedLangs = new Set<string>();

async function ensureLang(lang: HljsLang): Promise<void> {
  if (loadedLangs.has(lang)) return;
  const mod = (await LANG_LOADERS[lang]()) as { default: unknown };
  hljs.registerLanguage(lang, mod.default as never);
  loadedLangs.add(lang);
}

const cache = new Map<string, string>();
function keyOf(lang: HljsLang, code: string): string {
  return `${lang}|${code}`;
}

/** Highlight code with highlight.js. Returns an HTML string of `<span
 * class="hljs-…">` tokens (NO inline styles, NO wrapping `<pre>` — just the
 * inner `<code>` content). The caller injects it into `<pre><code>` via
 * dangerouslySetInnerHTML. Rejects if the language can't be loaded, in which
 * case the caller falls back to the regex highlighter. Results are cached. */
export async function highlightAsync(code: string, lang: HljsLang): Promise<string> {
  const k = keyOf(lang, code);
  const cached = cache.get(k);
  if (cached !== undefined) return cached;
  await ensureLang(lang);
  const html = hljs.highlight(code, { language: lang }).value;
  cache.set(k, html);
  return html;
}

// Keep the cache bounded so very long sessions don't grow it unbounded.
const CACHE_MAX = 256;
export function pruneCache(): void {
  if (cache.size > CACHE_MAX) {
    const keys = cache.keys();
    for (let i = 0; i < cache.size - CACHE_MAX; i++) {
      const r = keys.next();
      if (r.done) break;
      cache.delete(r.value);
    }
  }
}
