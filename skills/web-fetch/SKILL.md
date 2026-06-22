---
name: web-fetch
description: "Fetch live web pages for reading — docs, APIs, SPAs, localhost previews. Use for public URLs and dev-server HTML. Not for GitHub PR/CI data or repo source files."
argument-hint: "URL and what to extract"
intent_keywords: [web, url, http, page, site, preview, localhost, docs online, spa, javascript]
tools:
  - web_browser
---

# Web Fetch

Fetch pages the harness can render. Prefer the right tool for the job — GitHub PRs belong on harness/MCP, repo source on `read_file`.

## Scope

| Goal | Tool |
|------|------|
| Public URL or API JSON | `web_browser` |
| Local `dist/` or dev server | `web_browser` (`allow_localhost` for `localhost:PORT`) |
| JS-heavy SPA / anti-bot sites | `web_browser` with **`browser: true`** |
| `.tsx` / `.html` **in repo** | `read_file` |
| GitHub PR / CI | `pr_get_*`, `ci_get_*`, or `gh` — not HTML scrape |
| POST / custom headers | `bash_run curl` |

## Workflow

1. **Cheap pass** — `mode: metadata` on large or unknown pages.
2. **Full body** — `mode: full` when metadata is enough to proceed.
3. **Links only** — `mode: links` to explore doc sites.
4. **Blocked or empty** — retry **once** with `browser: true`; then stop looping.
5. **Still empty** — `read_file` on source or ask user to paste.

## Rules

- Do not scrape `github.com/.../pull/...` when PR tools exist.
- Ensure `chat.web_browser.allow_localhost: true` for local dev servers.

## Output template

### Page
Title, URL, mode used

### Content
Relevant excerpt or summary — not a full dump unless asked
