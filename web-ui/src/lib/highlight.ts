// Shiki-based async syntax highlighting. This module is deliberately heavy
// (it imports shiki/core + the JS regex engine + vscode-textmate) and is
// LAZY-LOADED by CodeBlock via a dynamic import() so none of it lands in the
// main entry chunk. A single highlighter core is created lazily and reused;
// per-(lang,theme) results are cached so repeated renders are instant.
//
// Theme follows next-themes: "github-dark" for dark, "github-light" for light.

import { createHighlighterCore, type HighlighterCore } from "shiki/core";
import { createJavaScriptRegexEngine } from "shiki/engine/javascript";
import type { ShikiLang, ShikiTheme } from "./lang";

// Re-export so existing imports from "../lib/highlight" keep working.
export type { ShikiLang, ShikiTheme } from "./lang";

// Dynamic imports — Vite code-splits each language/theme into its own chunk,
// loaded only when first needed.
const LANG_LOADERS: Record<ShikiLang, () => Promise<unknown>> = {
  bash: () => import("shiki/langs/bash.mjs"),
  shell: () => import("shiki/langs/shell.mjs"),
  json: () => import("shiki/langs/json.mjs"),
  rust: () => import("shiki/langs/rust.mjs"),
  javascript: () => import("shiki/langs/javascript.mjs"),
  typescript: () => import("shiki/langs/typescript.mjs"),
  python: () => import("shiki/langs/python.mjs"),
  go: () => import("shiki/langs/go.mjs"),
  yaml: () => import("shiki/langs/yaml.mjs"),
  sql: () => import("shiki/langs/sql.mjs"),
  toml: () => import("shiki/langs/toml.mjs"),
  diff: () => import("shiki/langs/diff.mjs"),
};

const THEME_LOADERS: Record<ShikiTheme, () => Promise<unknown>> = {
  "github-dark": () => import("shiki/themes/github-dark.mjs"),
  "github-light": () => import("shiki/themes/github-light.mjs"),
};

let corePromise: Promise<HighlighterCore> | null = null;
const loadedLangs = new Set<string>();
const loadedThemes = new Set<string>();

async function getCore(): Promise<HighlighterCore> {
  if (!corePromise) {
    // Use the JS regex engine (no oniguruma wasm) to keep the bundle small
    // and avoid a 600KB+ wasm chunk. Langs/themes are loaded on demand.
    corePromise = createHighlighterCore({
      engine: createJavaScriptRegexEngine(),
    });
  }
  return corePromise;
}

async function ensureLang(core: HighlighterCore, lang: ShikiLang): Promise<void> {
  if (loadedLangs.has(lang)) return;
  const mod = (await LANG_LOADERS[lang]()) as { default: unknown };
  await core.loadLanguage(mod.default as never);
  loadedLangs.add(lang);
}

async function ensureTheme(core: HighlighterCore, theme: ShikiTheme): Promise<void> {
  if (loadedThemes.has(theme)) return;
  const mod = (await THEME_LOADERS[theme]()) as { default: unknown };
  await core.loadTheme(mod.default as never);
  loadedThemes.add(theme);
}

const cache = new Map<string, string>();
function keyOf(lang: ShikiLang, theme: ShikiTheme, code: string): string {
  return `${lang}|${theme}|${code}`;
}

/** Highlight code with shiki. Returns the full `<pre class="shiki">…</pre>`
 * HTML (with inline styles for token colors AND the theme background). The
 * caller swaps this in for the fallback `<pre><code>` structure so shiki's
 * background and base foreground are preserved. Rejects if shiki isn't ready,
 * in which case the caller falls back to the regex highlighter. Results are
 * cached. */
export async function highlightAsync(
  code: string,
  lang: ShikiLang,
  theme: ShikiTheme,
): Promise<string> {
  const k = keyOf(lang, theme, code);
  const cached = cache.get(k);
  if (cached !== undefined) return cached;
  const core = await getCore();
  await Promise.all([ensureLang(core, lang), ensureTheme(core, theme)]);
  const html = core.codeToHtml(code, { lang, theme });
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
