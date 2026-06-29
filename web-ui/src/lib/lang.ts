// Dependency-free language resolution. Kept in the main chunk (tiny) so
// CodeBlock can decide synchronously whether a language is shiki-eligible
// without pulling the heavy shiki core (which is lazy-loaded separately).

export type ShikiTheme = "github-dark" | "github-light";

// Languages we highlight with shiki. Others fall back to the regex highlighter.
export const SHIKI_LANGS = [
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
  "toml",
  "diff",
] as const;
export type ShikiLang = (typeof SHIKI_LANGS)[number];

const LANG_ALIASES: Record<string, ShikiLang | undefined> = {
  sh: "bash",
  zsh: "bash",
  rs: "rust",
  js: "javascript",
  ts: "typescript",
  py: "python",
  yml: "yaml",
  golang: "go",
};

export function resolveLang(lang: string | undefined): ShikiLang | null {
  if (!lang) return null;
  const l = lang.toLowerCase();
  return (SHIKI_LANGS as readonly string[]).includes(l)
    ? (l as ShikiLang)
    : LANG_ALIASES[l] ?? null;
}
