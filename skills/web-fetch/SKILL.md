---
name: web-fetch
description: Fetch live web pages or built HTML for the agent to read (not for GitHub or repo source).
intent_keywords: [web, url, http, page, site, preview, localhost, docs online, zhihu, spa, javascript]
tools:
  - web_browser
---

## When to use `web_browser`

| Goal | Tool |
|------|------|
| Read a public URL or API JSON | `web_browser` |
| Preview `dist/index.html` or dev server | `web_browser` (`allow_localhost` for `localhost:PORT`) |
| JS-heavy SPA or sites with anti-scraping | `web_browser` with **`browser: true`** |
| Read `.tsx` / `.html` **source** in repo | `read_file` |
| GitHub PR / CI data | **Prefer** MCP (`pr_get_*`, `ci_get_*`) or `bash_run gh …`; web_browser HTML is a fallback only |
| POST / custom headers | `bash_run curl` |

## Modes

1. **`metadata`** — cheap first pass: title, description, headings, links.
2. **`links`** — link list only (explore a docs site).
3. **`full`** — metadata + body text (default).

## Browser mode (`browser: true`)

If the site uses anti-scraping (blocks bots, requires JS to render, or needs cookies/login in a real browser), retry with **`browser: true`** on the tool call — headless Chromium loads the page like a normal browser.

Optional tuning in `coworker.yaml`:

```yaml
chat:
  web_browser:
    browser_timeout_secs: 60
    browser_wait_ms: 3000
    # chromium_path: /Applications/Google Chrome.app/Contents/MacOS/Google Chrome
```

Example: `{ "url": "https://www.zhihu.com/question/123", "mode": "full", "browser": true }`

## Rules

- Prefer `mode=metadata` on large or unknown pages before `full`.
- If plain HTTP is blocked or the body is useless, retry once with `browser: true` — do not loop.
- If body is still empty after browser mode, use `read_file` on source or ask user to paste.
- For localhost dev servers, ensure `chat.web_browser.allow_localhost: true` in config.

## Anti-patterns

- Scraping github.com/pull/… when `pr_get_overview` or `gh pr view` is available.
- Using `web_browser` when `read_file` on local HTML source is enough.
- Fetching the same URL repeatedly without switching to `browser: true` when the site resists scraping.
