import type { Config } from "tailwindcss";

// Tailwind config: dark mode via `class` (driven by next-themes), content
// scanned from index.html and src/**. Colors map to the existing CSS variable
// design tokens so the React UI matches the legacy UI's Catppuccin-inspired
// palette without duplicating hex values.
export default {
  darkMode: "class",
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  theme: {
    extend: {
      colors: {
        bg: "var(--bg)",
        elevated: "var(--elevated)",
        text: "var(--text)",
        "text-muted": "var(--text-muted)",
        border: "var(--border)",
        accent: "var(--accent)",
        danger: "var(--danger)",
        warn: "var(--warn)",
        ok: "var(--ok)",
      },
      fontFamily: {
        mono: ["ui-monospace", "SFMono-Regular", "Menlo", "monospace"],
      },
    },
  },
  plugins: [],
} satisfies Config;
