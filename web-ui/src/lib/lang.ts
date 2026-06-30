// Dependency-free language resolution. Kept in the main chunk (tiny) so
// CodeBlock can decide synchronously whether a language is highlight-eligible
// without pulling the heavy highlight.js core (which is lazy-loaded separately).

export type HljsLang =
  | "bash"
  | "shell"
  | "json"
  | "rust"
  | "javascript"
  | "typescript"
  | "python"
  | "go"
  | "yaml"
  | "sql"
  | "ini"
  | "diff";

// Languages we highlight with highlight.js. Others fall back to the regex highlighter.
export const HLJS_LANGS: readonly HljsLang[] = [
  "bash",
  "shell",
  "json",
  "rust",
  "javascript",
  "typescript",
  "python",
  "go",
  "yaml",
  "sql",
  "ini",
  "diff",
];

// User-facing language name → highlight.js language module name.
// `toml` maps to `ini` (highlight.js has no standalone toml grammar; ini is
// close enough for key-value highlighting).
const LANG_ALIASES: Record<string, HljsLang | undefined> = {
  sh: "bash",
  zsh: "bash",
  rs: "rust",
  js: "javascript",
  ts: "typescript",
  py: "python",
  yml: "yaml",
  golang: "go",
  toml: "ini",
};

export function resolveLang(lang: string | undefined): HljsLang | null {
  if (!lang) return null;
  const l = lang.toLowerCase();
  if ((HLJS_LANGS as readonly string[]).includes(l)) return l as HljsLang;
  return LANG_ALIASES[l] ?? null;
}
