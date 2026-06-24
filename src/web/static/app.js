const THEME_STORAGE_KEY = "unistar-web-theme";

function getStoredTheme() {
  try {
    const t = localStorage.getItem(THEME_STORAGE_KEY);
    if (t === "light" || t === "dark") return t;
  } catch (_) {
    /* localStorage unavailable */
  }
  return null;
}

function initThemeFromConfig(configTheme) {
  applyTheme(getStoredTheme() || configTheme || "dark");
}

function applyTheme(theme) {
  document.documentElement.setAttribute("data-theme", theme);
  try {
    localStorage.setItem(THEME_STORAGE_KEY, theme);
  } catch (_) {
    /* localStorage unavailable */
  }
  const btn = document.getElementById("theme-toggle");
  if (!btn) return;
  const isLight = theme === "light";
  const label = isLight ? "Switch to dark mode" : "Switch to light mode";
  btn.setAttribute("aria-label", label);
  btn.title = label;
  btn.textContent = isLight ? "🌙" : "☀️";
}

function toggleTheme() {
  const current = document.documentElement.getAttribute("data-theme") || "dark";
  applyTheme(current === "light" ? "dark" : "light");
}

let state = null;
let prevState = null;
let ws = null;
let renderQueued = false;
let ui = {
  selectedDigestDate: null,
  selectedPrIndex: 0,
  chatStickBottom: true,
  ctxStickBottom: true,
  chatDraft: "",
  approvalDialogId: null,
  approvalArmAt: 0,
  approvalArmTimer: null,
  statusError: null,
  statusErrorTimer: null,
  expandedToolLines: new Set(),
  expandedReasoningGroups: new Set(),
  expandedToolGroups: new Set(),
  expandedCtxBlocks: new Set(),
  expandedCtxSkills: new Set(),
  expandedCtxTools: false,
  expandedCtxToolsChips: false,
  expandedCtxSkillsList: false,
  expandedToolBatches: new Set(),
  liveReasoningExpanded: false,
  lastStreamingPaint: 0,
  lastReasoningPaint: 0,
  lastHistoryRevision: null,
  lastContextRevision: null,
  lastHistoryLineCount: -1,
  sessionMenuOpen: false,
  sessionList: null,
  sessionListLoading: false,
  sessionMenuBound: false,
};

async function apiFetch(url, options = {}) {
  try {
    const res = await fetch(url, options);
    if (!res.ok) {
      const text = await res.text().catch(() => "");
      const msg = `${res.status} ${res.statusText}${text ? `: ${text.slice(0, 200)}` : ""}`;
      console.error(`API ${url}:`, msg);
      setStatusError(msg);
      return null;
    }
    return res;
  } catch (e) {
    console.error(`API ${url}:`, e);
    setStatusError(String(e.message || e));
    return null;
  }
}

function setStatusError(msg) {
  ui.statusError = msg;
  if (ui.statusErrorTimer) clearTimeout(ui.statusErrorTimer);
  ui.statusErrorTimer = setTimeout(() => {
    ui.statusError = null;
    ui.statusErrorTimer = null;
    updateStatus();
  }, 5000);
  updateStatus();
}

const TOOL_META = {
  bash_run: { icon: "⌘", label: "Bash" },
  python_run: { icon: "🐍", label: "Python" },
  web_fetch: { icon: "🌐", label: "Fetch" },
  web_browser: { icon: "🌐", label: "Fetch" },
  read_file: { icon: "📄", label: "Read" },
  write_file: { icon: "✎", label: "Write" },
  edit_file: { icon: "✎", label: "Edit" },
  grep: { icon: "🔍", label: "Grep" },
  glob: { icon: "📁", label: "Glob" },
  skill_search: { icon: "📚", label: "Skill search" },
  skill_load: { icon: "📚", label: "Load skill" },
  tool_search: { icon: "🔎", label: "Tool search" },
  tool_call: { icon: "⚡", label: "Tool call" },
  pr_get_diff: { icon: "⎇", label: "PR diff" },
  pr_get_overview: { icon: "◫", label: "PR overview" },
  pr_list_changed_files: { icon: "📋", label: "Changed files" },
  pr_diff_risk_scan: { icon: "⚠", label: "Diff risk" },
  pr_get_ci_snapshot: { icon: "◎", label: "CI snapshot" },
  pr_get_review_routing: { icon: "👥", label: "Review routing" },
  pr_get_review_state: { icon: "✓", label: "Review state" },
};

function toolMeta(name) {
  const key = (name || "").toLowerCase();
  const base = TOOL_META[key] || { icon: "⚙", label: name || "tool" };
  const source = toolSourceLabel(name);
  if (source) {
    return { ...base, source: source.source, sourceDetail: source.detail };
  }
  return base;
}

function toolSourceLabel(toolName) {
  if (!toolName) return null;
  const servers = state.mcp_servers || [];
  for (const s of servers) {
    const prefix = s.prefix || `${s.id}_`;
    if (toolName.startsWith(prefix)) {
      return {
        source: `mcp:${s.id}`,
        detail: toolName.slice(prefix.length) || toolName,
      };
    }
  }
  if (/^(pr_|ci_|issue_|repo_|release_|notify_)/.test(toolName)) {
    return { source: "github", detail: toolName };
  }
  if (
    ["bash_run", "python_run", "read_file", "write_file", "edit_file", "grep", "glob", "web_fetch", "web_browser", "skill_load", "skill_search"].includes(
      toolName,
    )
  ) {
    return { source: "local", detail: toolName };
  }
  return null;
}

function appendToolSourceChip(parent, meta) {
  if (!meta?.source) return;
  const chip = el("span", "tool-source-chip");
  chip.textContent = meta.sourceDetail ? `${meta.source} · ${meta.sourceDetail}` : meta.source;
  chip.title = `Tool backend: ${meta.source}`;
  parent.appendChild(chip);
}

function parseToolArgsString(args) {
  if (!args?.trim()) return [];
  const out = [];
  for (const part of args.split(",")) {
    const t = part.trim();
    if (!t) continue;
    const eq = t.indexOf("=");
    if (eq > 0) {
      out.push({ key: t.slice(0, eq).trim(), value: t.slice(eq + 1).trim() });
    } else {
      out.push({ key: t, value: "" });
    }
  }
  return out;
}

function formatToolArgValue(key, value) {
  const k = key.toLowerCase();
  if (!value) return "";
  if (k === "pr_number" || k === "pr") return `#${value}`;
  if (k === "max_bytes") {
    const n = Number.parseInt(value, 10);
    if (Number.isFinite(n) && n >= 1000) return `${Math.round(n / 1000)}k`;
    return value;
  }
  if (k === "repo") return truncateMiddle(value, 28);
  return truncateMiddle(value, 20);
}

function appendToolArgChips(parent, argsString) {
  const pairs = parseToolArgsString(argsString);
  if (!pairs.length) return;
  const chips = el("div", "tool-arg-chips");
  for (const { key, value } of pairs) {
    const chip = el("span", "tool-arg-chip");
    chip.appendChild(el("span", "tool-arg-k", key));
    const display = formatToolArgValue(key, value);
    if (display) chip.appendChild(el("span", "tool-arg-v", display));
    chips.appendChild(chip);
  }
  parent.appendChild(chips);
}

function buildToolCardHeader(meta, toolName, args, status, { pending = false, ms = null, collapsible = false } = {}) {
  const header = el("div", "tool-card-header" + (collapsible ? " clickable" : ""));
  header.appendChild(el("span", "tool-card-icon tool-glyph", meta.icon));
  const titleWrap = el("span", "tool-card-title-wrap");
  const displayTitle = meta.label || toolName;
  titleWrap.appendChild(el("span", "tool-card-title", displayTitle));
  if (toolName && displayTitle !== toolName) {
    titleWrap.appendChild(el("span", "tool-card-fn", toolName));
  }
  appendToolSourceChip(titleWrap, meta);
  if (args) {
    appendToolArgChips(titleWrap, args);
  } else if (status === "running" || status === "pending") {
    titleWrap.appendChild(
      el("span", "tool-card-status-hint", pending ? "Queued" : "Running…"),
    );
  }
  header.appendChild(titleWrap);
  const trail = el("span", "tool-card-trail");
  if (ms) trail.appendChild(el("span", "tool-card-ms", `${ms}ms`));
  if (status === "ok" || status === "err" || status === "pending") {
    const badge = status === "ok" ? "✓" : status === "err" ? "✗" : "⏳";
    trail.appendChild(el("span", `tool-status-badge status-${status}`, badge));
  }
  if (status === "running") trail.appendChild(el("span", "tool-spinner", ""));
  if (collapsible && status !== "running" && status !== "pending") {
    trail.appendChild(el("span", "tool-card-chevron", "▾"));
  }
  if (trail.childNodes.length) header.appendChild(trail);
  return header;
}

const PHASE_META = {
  tool: { label: "Running tool", cls: "phase-tool" },
  streaming: { label: "Writing reply", cls: "phase-streaming" },
  summarizing: { label: "Summarizing context", cls: "phase-summarizing" },
  reasoning: { label: "Reasoning", cls: "phase-reasoning" },
  activity: { label: "Loading skills", cls: "phase-activity" },
  model: { label: "Thinking", cls: "phase-model" },
};

const TAB_LABELS = {
  chat: "Chat",
  dashboard: "Dashboard",
  prs: "PRs",
  approvals: "Approvals",
  logs: "Logs",
  config: "Config",
};

const TAB_ICONS = {
  chat: "💬",
  dashboard: "📋",
  prs: "🔀",
  approvals: "✋",
  logs: "📜",
  config: "⚙️",
};

function el(tag, cls, html) {
  const n = document.createElement(tag);
  if (cls) n.className = cls;
  if (html != null) {
    if (html.includes("<")) n.innerHTML = html;
    else n.textContent = html;
  }
  return n;
}

function escapeHtml(s) {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function linkifyPlainText(text) {
  if (!text) return "";
  const escaped = escapeHtml(text);
  return escaped.replace(
    /(https?:\/\/[^\s<]+[^\s<.,;:!?)\]"'])/g,
    '<a href="$1" target="_blank" rel="noopener noreferrer">$1</a>',
  );
}

async function copyText(text, btn) {
  try {
    await navigator.clipboard.writeText(text);
    const orig = btn.textContent;
    btn.textContent = "Copied!";
    btn.classList.add("is-copied");
    setTimeout(() => {
      btn.textContent = orig;
      btn.classList.remove("is-copied");
    }, 1400);
  } catch (_) {
    btn.textContent = "Copy failed";
    setTimeout(() => {
      btn.textContent = "Copy";
    }, 1400);
  }
}

function autoResizeTextarea(ta) {
  if (!ta) return;
  ta.style.height = "auto";
  ta.style.height = `${Math.min(160, Math.max(42, ta.scrollHeight))}px`;
}

function phaseMeta(phase) {
  return PHASE_META[phase] || { label: phase || "Working", cls: "phase-model" };
}

function inlineMarkdown(s) {
  if (!s) return "";
  return s
    .replace(/\*\*(.+?)\*\*/g, "<strong>$1</strong>")
    .replace(/(?<!\*)\*(?!\*)(.+?)(?<!\*)\*(?!\*)/g, "<em>$1</em>")
    .replace(/(?<![\w])_([^_]+)_(?![\w])/g, "<em>$1</em>")
    .replace(/~~(.+?)~~/g, "<del>$1</del>")
    .replace(/`([^`]+)`/g, "<code>$1</code>")
    .replace(
      /\[([^\]]+)\]\(([^)]+)\)/g,
      '<a href="$2" target="_blank" rel="noopener noreferrer">$1</a>',
    );
}

function highlightCode(code, lang) {
  const L = (lang || "").toLowerCase();
  const str = (s) => `<span class="tok-string">${s}</span>`;
  const kw = (s) => `<span class="tok-kw">${s}</span>`;
  const cm = (s) => `<span class="tok-comment">${s}</span>`;
  const ky = (s) => `<span class="tok-key">${s}</span>`;

  if (L === "bash" || L === "sh" || L === "shell" || L === "zsh") {
    return code
      .replace(/(^|\n)(\s*#.*)/g, (_, prefix, comment) => `${prefix}${cm(comment)}`)
      .replace(/(&quot;[^&]*&quot;|'[^']*')/g, (m) => str(m))
      .replace(
        /\b(if|then|else|elif|fi|for|do|done|echo|cd|exit|export|source|sudo|curl|wget|grep)\b/g,
        (m) => kw(m),
      );
  }
  if (L === "json") {
    return code
      .replace(/(&quot;[^&]*&quot;)(\s*:)/g, (_, k, colon) => `${ky(k)}${colon}`)
      .replace(/:\s*(&quot;[^&]*&quot;)/g, (_, v) => `: ${str(v)}`)
      .replace(/\b(true|false|null)\b/g, (m) => kw(m));
  }
  if (L === "rust" || L === "rs") {
    const kws =
      "fn|let|mut|pub|use|struct|enum|impl|match|if|else|return|async|await|true|false|Some|None|Ok|Err";
    return code
      .replace(/(\/\/.*)/g, (m) => cm(m))
      .replace(/(&quot;[^&]*&quot;)/g, (m) => str(m))
      .replace(new RegExp(`\\b(${kws})\\b`, "g"), (m) => kw(m));
  }
  if (L === "javascript" || L === "js" || L === "typescript" || L === "ts") {
    const kws =
      "function|const|let|var|return|if|else|async|await|import|export|from|true|false|null|undefined|class|new";
    return code
      .replace(/(\/\/.*)/g, (m) => cm(m))
      .replace(/(&quot;[^&]*&quot;|`[^`]*`|'[^']*')/g, (m) => str(m))
      .replace(new RegExp(`\\b(${kws})\\b`, "g"), (m) => kw(m));
  }
  return code;
}

function parseTableBlock(lines, start) {
  const rows = [];
  let i = start;
  while (i < lines.length && lines[i].includes("|")) {
    rows.push(lines[i]);
    i++;
  }
  if (rows.length < 2) return null;
  const parseRow = (r) => {
    const parts = r.trim().split("|").map((c) => c.trim());
    if (parts[0] === "") parts.shift();
    if (parts[parts.length - 1] === "") parts.pop();
    return parts;
  };
  if (!/^[\|\s\-:]+$/.test(rows[1])) return null;
  const header = parseRow(rows[0]).map(inlineMarkdown);
  const bodyRows = rows.slice(2).map(parseRow);
  let html =
    "<div class=\"md-table-wrap\"><table><thead><tr>" +
    header.map((h) => `<th>${h}</th>`).join("") +
    "</tr></thead><tbody>";
  for (const row of bodyRows) {
    html += "<tr>" + row.map((c) => `<td>${inlineMarkdown(c)}</td>`).join("") + "</tr>";
  }
  return { html: html + "</tbody></table></div>", next: i };
}

function isOrderedListLine(line) {
  return /^\d+\.\s?/.test(line);
}

function isBulletListLine(line) {
  return /^\s*[-*] /.test(line);
}

function bulletListItemText(line) {
  const m = line.match(/^\s*[-*]\s+(.*)$/);
  return m ? m[1] : line;
}

/** Headings / tables end an ordered-list region; fences and paragraphs may appear between items. */
function endsOrderedListRegion(lines, i, isBlank) {
  if (i >= lines.length) return true;
  let j = i;
  while (j < lines.length && isBlank(lines[j])) j += 1;
  if (j >= lines.length) return true;
  const line = lines[j];
  if (/^#{1,6}\s/.test(line)) return true;
  if (/^-{3,}$/.test(line.trim())) return true;
  if (line.startsWith("&gt;")) return true;
  if (
    line.includes("|") &&
    j + 1 < lines.length &&
    lines[j + 1].includes("|")
  ) {
    return true;
  }
  return false;
}

/** Ordered list with sub-bullets, code fences, and lazy `1.` numbering (PR findings). */
function parseOrderedListBlock(lines, start, isBlank, isFence) {
  if (!isOrderedListLine(lines[start])) return null;

  const items = [];
  let i = start;

  const nextSignificant = (from) => {
    let j = from;
    while (j < lines.length && isBlank(lines[j])) j += 1;
    return j;
  };

  while (i < lines.length) {
    if (endsOrderedListRegion(lines, i, isBlank)) break;

    while (i < lines.length) {
      if (endsOrderedListRegion(lines, i, isBlank)) break;
      if (isBlank(lines[i])) {
        i += 1;
        continue;
      }
      if (isFence(lines[i])) {
        if (items.length) items[items.length - 1] += lines[i].trim();
        i += 1;
        continue;
      }
      if (isOrderedListLine(lines[i])) break;
      if (items.length) {
        items[items.length - 1] += `<br>${inlineMarkdown(lines[i])}`;
      }
      i += 1;
    }

    if (i >= lines.length || !isOrderedListLine(lines[i])) break;

    let itemHtml = inlineMarkdown(lines[i].replace(/^\d+\.\s?/, ""));
    i += 1;
    const subBullets = [];

    while (i < lines.length) {
      if (isBlank(lines[i])) {
        const j = nextSignificant(i);
        if (j >= lines.length) {
          i = j;
          break;
        }
        if (isOrderedListLine(lines[j])) {
          i = j;
          break;
        }
        if (endsOrderedListRegion(lines, i, isBlank)) break;
        i += 1;
        continue;
      }
      if (isOrderedListLine(lines[i])) break;
      if (endsOrderedListRegion(lines, i, isBlank)) break;
      if (isFence(lines[i])) {
        itemHtml += lines[i].trim();
        i += 1;
        continue;
      }
      if (isBulletListLine(lines[i])) {
        subBullets.push(bulletListItemText(lines[i]));
        i += 1;
        continue;
      }
      itemHtml += `<br>${inlineMarkdown(lines[i])}`;
      i += 1;
    }

    if (subBullets.length) {
      itemHtml +=
        "<ul>" +
        subBullets.map((b) => `<li>${inlineMarkdown(b)}</li>`).join("") +
        "</ul>";
    }
    items.push(`<li>${itemHtml}</li>`);
  }

  if (!items.length) return null;
  return { html: `<ol class="md-ol">${items.join("")}</ol>`, next: i };
}

/** Merge fragmented `<ol><li>…</li></ol>` runs (safety net after loose parsing). */
function mergeAdjacentOrderedLists(html) {
  const re = /<ol class="md-ol">(\s*<li>[\s\S]*?<\/li>\s*)<\/ol>/g;
  let out = html;
  let prev;
  let guard = 0;
  do {
    prev = out;
    out = out.replace(
      /(<ol class="md-ol">\s*<li>[\s\S]*?<\/li>\s*<\/ol>\s*){2,}/g,
      (chunk) => {
        const lis = chunk.match(/<li>[\s\S]*?<\/li>/g) || [];
        return `<ol class="md-ol">${lis.join("")}</ol>`;
      },
    );
    guard += 1;
  } while (out !== prev && guard < 24);
  return out.replace(re, '<ol class="md-ol">$1</ol>');
}

/** Plain text for in-progress assistant stream (avoid full markdown each token). */
function streamingPlainHtml(text) {
  if (!text) return "";
  return escapeHtml(text).replace(/\n/g, "<br>");
}

function scrollLiveBodyIfNeeded(body) {
  if (!body) return;
  const gap = body.scrollHeight - body.scrollTop - body.clientHeight;
  if (ui.chatStickBottom || gap < 96) {
    body.scrollTop = body.scrollHeight;
  }
}

function paintStreamingBody(body, text) {
  const now = performance.now();
  if (now - ui.lastStreamingPaint < 80 && body.textContent?.length) {
    const delta = text.length - (body.dataset.streamLen || 0);
    if (delta < 32 && delta >= 0) return;
  }
  ui.lastStreamingPaint = now;
  body.dataset.streamLen = String(text.length);
  body.innerHTML = streamingPlainHtml(text);
  scrollLiveBodyIfNeeded(body);
}

const REASONING_COLLAPSE_LINES = 6;
const REASONING_COLLAPSE_CHARS = 420;

function normalizeReasoningText(text) {
  if (!text) return "";
  let s = String(text).trim();
  s = s.replace(/^\[agent reasoning summary\]\s*/i, "");
  s = s.replace(/^reasoning:\s*/i, "");
  return s.trim();
}

function reasoningLineCount(text) {
  const n = normalizeReasoningText(text);
  if (!n) return 0;
  return n.split("\n").filter((line) => line.trim()).length;
}

function reasoningCharCount(text) {
  return normalizeReasoningText(text).length;
}

function isReasoningLong(text) {
  const n = normalizeReasoningText(text);
  if (!n) return false;
  const lines = n.split("\n");
  return lines.length > REASONING_COLLAPSE_LINES || n.length > REASONING_COLLAPSE_CHARS;
}

function reasoningPreviewHtml(text) {
  const normalized = normalizeReasoningText(text);
  if (!normalized) return "";
  let clipped = normalized.split("\n").slice(0, REASONING_COLLAPSE_LINES).join("\n");
  if (clipped.length > REASONING_COLLAPSE_CHARS) {
    clipped = `${clipped.slice(0, REASONING_COLLAPSE_CHARS)}…`;
  }
  return `<div class="reasoning-plain preview">${escapeHtml(clipped)}</div>`;
}

function fillReasoningBody(bodyEl, text, { live = false, expanded = true } = {}) {
  const normalized = normalizeReasoningText(text);
  if (!normalized) {
    bodyEl.innerHTML = live
      ? '<span class="reasoning-empty">Thinking…</span>'
      : '<span class="reasoning-empty">No reasoning captured.</span>';
    bodyEl.classList.toggle("is-live", live);
    return;
  }
  let inner;
  if (live) {
    inner = `<div class="reasoning-plain is-live">${streamingPlainHtml(normalized)}</div>`;
  } else if (!expanded && isReasoningLong(text)) {
    inner = reasoningPreviewHtml(text);
  } else {
    inner = `<div class="reasoning-md md">${renderMarkdown(normalized)}</div>`;
  }
  bodyEl.innerHTML = `${inner}${live ? '<span class="reasoning-cursor" aria-hidden="true"></span>' : ""}`;
  bodyEl.classList.toggle("is-live", live);
  if (live) scrollLiveBodyIfNeeded(bodyEl);
}

function paintReasoningBody(body, text, expanded = true) {
  const now = performance.now();
  if (now - ui.lastReasoningPaint < 120 && body.dataset.reasoningLen) {
    const delta = text.length - Number(body.dataset.reasoningLen || 0);
    if (delta < 40 && delta >= 0) return;
  }
  ui.lastReasoningPaint = now;
  body.dataset.reasoningLen = String(text.length);
  fillReasoningBody(body, text, { live: true, expanded });
}

function updateReasoningMeta(card, text, { live = false } = {}) {
  const meta = card.querySelector(".activity-reasoning-meta");
  if (!meta) return;
  const normalized = normalizeReasoningText(text);
  if (!normalized) {
    meta.textContent = live ? "streaming…" : "";
    return;
  }
  meta.textContent = `${reasoningLineCount(text)} lines · ${reasoningCharCount(text).toLocaleString()} chars`;
}

function buildReasoningCard(text, { live = false, expanded, onToggle } = {}) {
  const long = isReasoningLong(text);
  const isExpanded = expanded ?? (!long || live);
  const card = el(
    "div",
    "activity-reasoning" +
      (live ? " is-live" : " history-reasoning") +
      (long && !isExpanded ? " collapsed" : ""),
  );

  const head = el("div", "activity-reasoning-head");
  head.appendChild(el("span", "activity-icon", "💭"));
  const titleWrap = el("div", "activity-reasoning-title-wrap");
  titleWrap.appendChild(el("span", "activity-title", "Reasoning"));
  const meta = el("span", "activity-reasoning-meta", live && !text ? "streaming…" : "");
  if (text && normalizeReasoningText(text)) {
    meta.textContent = `${reasoningLineCount(text)} lines · ${reasoningCharCount(text).toLocaleString()} chars`;
  }
  titleWrap.appendChild(meta);
  head.appendChild(titleWrap);

  if (long) {
    const btn = el(
      "button",
      "activity-toggle",
      isExpanded ? "Collapse" : live ? "Expand" : "Show all",
    );
    btn.type = "button";
    btn.onclick = (e) => {
      e.stopPropagation();
      e.preventDefault();
      onToggle?.();
    };
    head.appendChild(btn);
  }
  card.appendChild(head);

  const body = el("div", "activity-reasoning-body");
  fillReasoningBody(body, text, { live, expanded: isExpanded });
  card.appendChild(body);
  return card;
}

function resolveReasoningFullText(block) {
  if (block.fullText) return block.fullText;
  const indices = block.stepIndices || (block.index != null ? [block.index] : []);
  const texts = block.texts || [];
  const parts = indices.map((idx, i) => {
    const stored = getToolOutput(idx);
    if (stored) return normalizeReasoningText(stored);
    return normalizeReasoningText(texts[i] || "");
  });
  return parts.filter(Boolean).join("\n\n");
}

/** Lightweight markdown → HTML (assistant / digest / overview). */
const MARKDOWN_MAX_CHARS = 24_000;

function renderMarkdown(text) {
  if (!text) return "";
  if (text.length > MARKDOWN_MAX_CHARS) {
    const cut = text.slice(0, MARKDOWN_MAX_CHARS);
    return `<pre class="md-plain">${escapeHtml(cut)}\n… [${text.length - MARKDOWN_MAX_CHARS} more chars]</pre>`;
  }

  const fences = [];
  let src = text.replace(/```(\w*)\n?([\s\S]*?)```/g, (_, lang, code) => {
    const i = fences.length;
    const trimmed = code.trimEnd();
    const safeLang = escapeHtml(lang || "text");
    const highlighted = highlightCode(escapeHtml(trimmed), lang);
    const langBadge = lang ? `<span class="md-code-lang">${safeLang}</span>` : "";
    fences.push(
      `<div class="md-code-block">${langBadge}<pre><code class="lang-${safeLang}">${highlighted}</code></pre></div>`,
    );
    return `\x00FENCE${i}\x00`;
  });

  src = escapeHtml(src);
  const lines = src.split("\n");
  const out = [];
  let i = 0;

  const isBlank = (l) => !l.trim();
  const isFence = (l) => /^\x00FENCE\d+\x00$/.test(l.trim());

  while (i < lines.length) {
    if (isBlank(lines[i])) {
      i++;
      continue;
    }

    if (isFence(lines[i])) {
      out.push(lines[i].trim());
      i++;
      continue;
    }

    if (/^-{3,}$/.test(lines[i].trim())) {
      out.push("<hr>");
      i++;
      continue;
    }

    const hm = lines[i].match(/^(#{1,6})\s+(.+)$/);
    if (hm) {
      const level = hm[1].length;
      out.push(`<h${level}>${inlineMarkdown(hm[2])}</h${level}>`);
      i++;
      continue;
    }

    if (lines[i].startsWith("&gt;")) {
      const quoteLines = [];
      while (i < lines.length && lines[i].startsWith("&gt;")) {
        quoteLines.push(lines[i].replace(/^&gt; ?/, ""));
        i++;
      }
      const inner = quoteLines.map((l) => inlineMarkdown(l)).join("<br>");
      out.push(`<blockquote><p>${inner}</p></blockquote>`);
      continue;
    }

    if (lines[i].includes("|") && i + 1 < lines.length && lines[i + 1].includes("|")) {
      const table = parseTableBlock(lines, i);
      if (table) {
        out.push(table.html);
        i = table.next;
        continue;
      }
    }

    if (isBulletListLine(lines[i])) {
      const items = [];
      while (i < lines.length && isBulletListLine(lines[i])) {
        let item = bulletListItemText(lines[i]);
        const taskM = item.match(/^\[([ xX])\]\s*(.*)$/);
        if (taskM) {
          const checked = taskM[1] !== " ";
          items.push(
            `<li class="task${checked ? " done" : ""}"><span class="md-task" aria-hidden="true">${checked ? "☑" : "☐"}</span> ${inlineMarkdown(taskM[2])}</li>`,
          );
        } else {
          items.push(`<li>${inlineMarkdown(item)}</li>`);
        }
        i++;
      }
      out.push(`<ul>${items.join("")}</ul>`);
      continue;
    }

    if (/^\d+\.\s?/.test(lines[i])) {
      const block = parseOrderedListBlock(lines, i, isBlank, isFence);
      if (block) {
        out.push(block.html);
        i = block.next;
        continue;
      }
    }

    const para = [];
    while (
      i < lines.length &&
      !isBlank(lines[i]) &&
      !isFence(lines[i]) &&
      !/^#{1,6}\s/.test(lines[i]) &&
      !/^-{3,}$/.test(lines[i].trim()) &&
      !lines[i].startsWith("&gt;") &&
      !isBulletListLine(lines[i]) &&
      !/^\d+\.\s?/.test(lines[i]) &&
      !(lines[i].includes("|") && i + 1 < lines.length && lines[i + 1].includes("|"))
    ) {
      para.push(lines[i]);
      i++;
    }
    if (para.length) {
      out.push(`<p>${inlineMarkdown(para.join("<br>"))}</p>`);
    }
  }

  const html = out.join("\n").replace(/\x00FENCE(\d+)\x00/g, (_, idx) => fences[Number(idx)]);
  return mergeAdjacentOrderedLists(html);
}

function parseMessage(line) {
  if (line.startsWith("you> ")) return { role: "you", badge: "You", body: line.slice(5) };
  if (line.startsWith("assistant> ")) return { role: "assistant", badge: "AI", body: line.slice(11), md: true };
  if (line.startsWith("system> ")) return { role: "system", badge: "system", body: line.slice(8) };
  if (line.startsWith("error> ")) return { role: "error", badge: "error", body: line.slice(7) };
  if (line.startsWith("  ✓ ")) return { role: "tool", badge: "✓", body: line.slice(4), icon: "ok" };
  if (line.startsWith("  → ")) return { role: "tool", badge: "→", body: line.slice(4), icon: "run" };
  if (line.startsWith("  ✗ ")) return { role: "tool", badge: "✗", body: line.slice(4), icon: "err" };
  if (line.startsWith("  ⚠ ")) return { role: "tool", badge: "⚠", body: line.slice(4), icon: "warn" };
  if (line.startsWith("  ⏳ ")) return { role: "tool", badge: "⏳", body: line.slice(4), icon: "pending" };
  if (line.startsWith("  … ")) return { role: "tool", badge: "…", body: line.slice(4), icon: "reasoning" };
  if (line.startsWith("chat> ")) return { role: "system", badge: "chat", body: line.slice(6) };
  return { role: "system", badge: "·", body: line };
}

function getToolOutput(lineIndex) {
  const outs = state?.chat_tool_outputs;
  if (!outs) return null;
  return outs[String(lineIndex)] ?? outs[lineIndex] ?? null;
}

function splitToolCall(body) {
  const m = body.match(/^([\w.-]+)(?:\((.+)\))?$/);
  return { name: m?.[1] || body, args: m?.[2] || null };
}

function splitToolDone(body) {
  const msM = body.match(/\((\d+)ms\)\s*$/);
  const ms = msM ? msM[1] : null;
  const rest = msM ? body.slice(0, msM.index).trim() : body;
  const call = splitToolCall(rest);
  return { ...call, ms };
}

function parseToolStep(line, index) {
  const output = getToolOutput(index);
  if (line.startsWith("  → ")) {
    const body = line.slice(4);
    return { kind: "start", text: body, index, ...splitToolCall(body) };
  }
  if (line.startsWith("  ⏳ ")) {
    const body = line.slice(4);
    return { kind: "approval-pending", text: body, index, ok: null };
  }
  if (line.startsWith("  ✓ ") || line.startsWith("  ✗ ")) {
    const ok = line.startsWith("  ✓ ");
    const body = line.slice(4);
    if (/^approval (resolved|approved|denied|failed)/i.test(body)) {
      return { kind: "approval", text: body, index, ok };
    }
    return { kind: "done", text: body, index, ok, output, ...splitToolDone(body) };
  }
  if (line.startsWith("  ⚠ ")) {
    return { kind: "warn", text: line.slice(4), index };
  }
  if (line.startsWith("  … ")) {
    const raw = line.slice(4);
    const stored = output ? normalizeReasoningText(output) : null;
    return { kind: "reasoning", text: raw, fullText: stored, index };
  }
  if (line.startsWith("chat> ")) {
    return { kind: "meta", text: line.slice(6), badge: "chat", index };
  }
  const p = parseMessage(line);
  return { kind: "meta", text: p.body, badge: p.badge, index };
}

function isPrimaryBlock(parsed) {
  return parsed.role === "you" || parsed.role === "assistant" || parsed.role === "error";
}

function summarizeToolGroup(steps) {
  const named = steps.find((s) => s.name);
  const toolName = named?.name || steps.find((s) => s.kind === "start")?.name || "tool";
  const done = [...steps].reverse().find((s) => s.kind === "done");
  const pending = steps.some((s) => s.kind === "approval-pending");
  let status = "neutral";
  if (done) status = done.ok ? "ok" : "err";
  else if (pending) status = "pending";
  else if (steps.some((s) => s.kind === "start")) status = "running";
  else if (steps.some((s) => s.kind === "warn")) status = "warn";
  const ms = done?.ms || null;
  const args = done?.args || steps.find((s) => s.args)?.args || null;
  return { toolName, status, ms, args };
}

/** Pair tool start/done rows — parallel tools interleave `→` before all `✓`. */
function splitToolStepGroups(steps) {
  const pending = [];
  const groups = [];

  function pushGroup(stepList) {
    if (stepList.length) groups.push(stepList);
  }

  function firstPendingStartIndex(toolName) {
    if (!toolName) return -1;
    for (let i = 0; i < pending.length; i++) {
      const start = pending[i].steps.find((s) => s.kind === "start");
      if (start?.name === toolName) return i;
    }
    return -1;
  }

  for (const step of steps) {
    if (step.kind === "start") {
      pending.push({ steps: [step] });
      continue;
    }
    if (step.kind === "done") {
      const matchIdx = firstPendingStartIndex(step.name);
      if (matchIdx >= 0) {
        const group = pending.splice(matchIdx, 1)[0];
        group.steps.push(step);
        pushGroup(group.steps);
      } else {
        pushGroup([step]);
      }
      continue;
    }
    if (step.kind === "approval") {
      const matchIdx = firstPendingStartIndex(splitToolCall(step.text).name);
      if (matchIdx >= 0) {
        pending[matchIdx].steps.push(step);
      } else if (pending.length) {
        pending[pending.length - 1].steps.push(step);
      } else {
        pushGroup([step]);
      }
      continue;
    }
    if (pending.length) {
      pending[pending.length - 1].steps.push(step);
    } else if (groups.length) {
      groups[groups.length - 1].push(step);
    } else {
      pushGroup([step]);
    }
  }

  for (const g of pending) {
    pushGroup(g.steps);
  }
  return groups;
}

function pushToolStepBlocks(blocks, steps) {
  if (!steps.length) return;
  if (steps.every((s) => s.kind === "reasoning")) {
    blocks.push({
      type: "reasoning",
      texts: steps.map((s) => s.text),
      stepIndices: steps.map((s) => s.index),
      fullText: steps
        .map((s) => s.fullText || getToolOutput(s.index))
        .filter(Boolean)
        .map(normalizeReasoningText)
        .join("\n\n"),
      index: steps[0].index,
    });
    return;
  }
  if (steps.length === 1 && steps[0].kind === "meta") {
    blocks.push({ type: "system", body: steps[0].text, index: steps[0].index });
    return;
  }
  for (const groupSteps of splitToolStepGroups(steps)) {
    if (groupSteps.every((s) => s.kind === "reasoning")) {
      blocks.push({
        type: "reasoning",
        texts: groupSteps.map((s) => s.text),
        stepIndices: groupSteps.map((s) => s.index),
        fullText: groupSteps
          .map((s) => s.fullText || getToolOutput(s.index))
          .filter(Boolean)
          .map(normalizeReasoningText)
          .join("\n\n"),
        index: groupSteps[0].index,
      });
    } else if (groupSteps.length === 1 && groupSteps[0].kind === "meta") {
      blocks.push({ type: "system", body: groupSteps[0].text, index: groupSteps[0].index });
    } else {
      blocks.push({ type: "tool-group", steps: groupSteps, ...summarizeToolGroup(groupSteps) });
    }
  }
}

function buildMessageBlocks(lines) {
  const blocks = [];
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    if (!line.trim()) {
      i++;
      continue;
    }
    const parsed = parseMessage(line);
    if (isPrimaryBlock(parsed)) {
      blocks.push({ type: parsed.role, body: parsed.body, md: parsed.md, index: i });
      i++;
      continue;
    }
    if (line.startsWith("chat> ")) {
      blocks.push({ type: "system", body: line.slice(6), index: i });
      i++;
      continue;
    }
    const steps = [];
    while (i < lines.length) {
      const l = lines[i];
      if (!l.trim()) {
        i++;
        continue;
      }
      if (l.startsWith("you> ") || l.startsWith("assistant> ") || l.startsWith("error> ")) break;
      if (l.startsWith("chat> ")) break;
      if (l.startsWith("system> ")) {
        if (steps.length) {
          pushToolStepBlocks(blocks, steps);
          steps.length = 0;
        }
        blocks.push({ type: "system", body: l.slice(8), index: i });
        i++;
        continue;
      }
      steps.push(parseToolStep(l, i));
      i++;
    }
    pushToolStepBlocks(blocks, steps);
  }
  return blocks;
}

/** Merge 3+ consecutive completed tool groups into one compact strip. */
function mergeConsecutiveToolGroups(blocks) {
  const out = [];
  let run = [];
  const flush = () => {
    if (!run.length) return;
    const batchId = `tb-${run[0].steps?.[0]?.index ?? run[0].index ?? 0}-${run.length}`;
    if (run.length >= 3) {
      out.push({ type: "tool-batch", groups: [...run], batchId, index: run[0].index });
    } else {
      for (const g of run) out.push(g);
    }
    run = [];
  };
  for (let i = 0; i < blocks.length; i++) {
    const b = blocks[i];
    const batchable =
      b.type === "tool-group" &&
      (b.status === "ok" || b.status === "err" || b.status === "warn" || b.status === "neutral");
    if (batchable) run.push(b);
    else {
      flush();
      out.push(b);
    }
  }
  flush();
  return out;
}

function toolBatchFingerprint(block) {
  return JSON.stringify({
    t: "tb",
    id: block.batchId,
    expanded: ui.expandedToolBatches.has(block.batchId),
    groups: block.groups.map((g, i) => blockFingerprint(g, blockDomId(g, i))),
  });
}

function renderToolBatch(parent, block) {
  const expanded = ui.expandedToolBatches.has(block.batchId);
  const ok = block.groups.filter((g) => g.status === "ok").length;
  const err = block.groups.filter((g) => g.status === "err").length;
  const strip = el("div", "tool-run-strip" + (expanded ? " is-expanded" : ""));
  const summary = el("button", "tool-run-summary", "");
  summary.type = "button";
  const labels = block.groups.map((g) => toolMeta(g.toolName).label).join(" · ");
  summary.title = labels;
  summary.innerHTML = `<span class="tool-run-count">${block.groups.length} tools</span>` +
    `<span class="tool-run-labels">${escapeHtml(truncateMiddle(labels, 72))}</span>` +
    `<span class="tool-run-stats">${ok ? `<span class="ok">${ok}✓</span>` : ""}${err ? `<span class="err">${err}✗</span>` : ""}</span>` +
    `<span class="tool-run-chevron">${expanded ? "▾" : "▸"}</span>`;
  summary.onclick = (e) => {
    e.stopPropagation();
    if (ui.expandedToolBatches.has(block.batchId)) {
      ui.expandedToolBatches.delete(block.batchId);
      block.groups.forEach((g, i) => ui.expandedToolGroups.delete(blockDomId(g, i)));
    } else {
      ui.expandedToolBatches.add(block.batchId);
    }
    const node = parent.closest("[data-block-id]");
    if (node) delete node.dataset.fp;
    scheduleRender(true);
  };
  strip.appendChild(summary);
  if (expanded) {
    const list = el("div", "tool-run-list");
    block.groups.forEach((g, i) => {
      const wrap = el("div", "tool-run-item");
      renderToolGroup(wrap, g, blockDomId(g, i));
      list.appendChild(wrap);
    });
    strip.appendChild(list);
  } else {
    const chips = el("div", "tool-run-chips");
    block.groups.forEach((g, i) => {
      const domId = blockDomId(g, i);
      const chip = el("span", `tool-run-chip status-${g.status}`);
      chip.title = g.args || g.toolName;
      chip.textContent = toolMeta(g.toolName).label;
      chip.onclick = (e) => {
        e.stopPropagation();
        ui.expandedToolGroups.add(domId);
        ui.expandedToolBatches.add(block.batchId);
        const node = parent.closest("[data-block-id]");
        if (node) delete node.dataset.fp;
        scheduleRender(true);
      };
      chips.appendChild(chip);
    });
    strip.appendChild(chips);
  }
  parent.appendChild(strip);
}

const STEP_ICONS = {
  start: "▶",
  done: "✓",
  "approval-pending": "⏳",
  approval: "✋",
  warn: "⚠",
  reasoning: "💭",
  meta: "·",
};

function stepIcon(step) {
  if (step.kind === "done") return step.ok ? "✓" : "✗";
  if (step.kind === "approval") return step.ok ? "✓" : "✗";
  return STEP_ICONS[step.kind] || "·";
}

function formatStepText(step) {
  if (step.kind === "start" && step.args) {
    return `${step.name}(${truncateMiddle(step.args, 72)})`;
  }
  if (step.kind === "done" && step.args) {
    return `${step.name}(${truncateMiddle(step.args, 72)})`;
  }
  return step.text;
}

function isToolExpanded(blockId) {
  return ui.expandedToolGroups.has(blockId);
}

function shouldCompactTool(block, blockId) {
  if (block.status === "running" || block.status === "pending") return false;
  return !isToolExpanded(blockId);
}

function toolOutputSummary(block) {
  const step = block.steps.find((s) => s.output);
  if (!step?.output) return null;
  const lines = step.output.split("\n").length;
  if (lines > 1) return `${lines} lines`;
  if (step.output.length > 80) return `${formatTokens(step.output.length)} chars`;
  return null;
}

function messageStatsFromLines(lines) {
  const blocks = mergeConsecutiveToolGroups(buildMessageBlocks(lines || []));
  const toolCount =
    blocks.filter((b) => b.type === "tool-group").length +
    blocks.filter((b) => b.type === "tool-batch").reduce((n, b) => n + (b.groups?.length || 0), 0);
  return {
    blocks: blocks.length,
    you: blocks.filter((b) => b.type === "you").length,
    ai: blocks.filter((b) => b.type === "assistant").length,
    tools: toolCount,
    reasoning: blocks.filter((b) => b.type === "reasoning").length,
  };
}

function formatMessageCount(stats) {
  if (!stats.blocks) return "";
  const parts = [`${stats.blocks} blocks`];
  if (stats.you) parts.push(`${stats.you} you`);
  if (stats.ai) parts.push(`${stats.ai} ai`);
  if (stats.tools) parts.push(`${stats.tools} tools`);
  return parts.join(" · ");
}

function truncateMiddle(s, max) {
  if (!s || s.length <= max) return s;
  const half = Math.floor((max - 1) / 2);
  return s.slice(0, half) + "…" + s.slice(-half);
}

function roleClass(role) {
  const r = (role || "").toLowerCase();
  if (r.includes("user") || r === "you") return "role-user";
  if (r.includes("assistant") || r === "ai") return "role-assistant";
  if (r.includes("tool")) return "role-tool";
  if (r.includes("system")) return "role-system";
  return "role-other";
}

/** Highlight common bash/tool output patterns. */
function formatToolOutputHtml(text) {
  const lines = text.split("\n");
  return lines
    .map((line) => {
      const esc = escapeHtml(line);
      if (/^exit:\s*\d+/i.test(line)) {
        const ok = /exit:\s*0\b/i.test(line);
        return `<span class="out-line out-exit ${ok ? "ok" : "err"}">${esc}</span>`;
      }
      if (/^stderr:/i.test(line)) return `<span class="out-line out-stderr">${esc}</span>`;
      if (/^stdout:/i.test(line)) return `<span class="out-line out-stdout">${esc}</span>`;
      if (/^cwd:/i.test(line)) return `<span class="out-line out-meta">${esc}</span>`;
      if (/error|failed|invalid/i.test(line)) return `<span class="out-line out-err">${esc}</span>`;
      return `<span class="out-line">${esc}</span>`;
    })
    .join("\n");
}

function toolOutputKey(domId, lineIndex) {
  return `${domId}:${lineIndex}`;
}

function renderToolOutput(wrap, output, lineIndex, domId) {
  const key = toolOutputKey(domId, lineIndex);
  const lines = output.split("\n");
  const collapsible = lines.length > 6 || output.length > 480;
  const expanded = ui.expandedToolLines.has(key);

  wrap.className = "tool-output-wrap" + (expanded && collapsible ? " is-expanded" : "");
  wrap.replaceChildren();

  const pre = el("pre", "tool-output");
  pre.innerHTML = formatToolOutputHtml(
    collapsible && !expanded ? lines.slice(0, 5).join("\n") + "\n…" : output,
  );
  wrap.appendChild(pre);

  if (collapsible) {
    const actions = el("div", "tool-output-actions");
    const btn = el(
      "button",
      "tool-output-toggle",
      expanded ? "Collapse output" : `Show all ${lines.length} lines`,
    );
    btn.type = "button";
    btn.onclick = (e) => {
      e.stopPropagation();
      e.preventDefault();
      if (expanded) ui.expandedToolLines.delete(key);
      else ui.expandedToolLines.add(key);
      renderToolOutput(wrap, output, lineIndex, domId);
      const card = wrap.closest("[data-block-id]");
      if (card) delete card.dataset.fp;
      scheduleRender(true);
    };
    actions.appendChild(btn);
    if (expanded) {
      const copyBtn = el("button", "tool-output-copy", "Copy");
      copyBtn.type = "button";
      copyBtn.onclick = (e) => {
        e.stopPropagation();
        e.preventDefault();
        copyText(output, copyBtn);
      };
      actions.appendChild(copyBtn);
    }
    wrap.appendChild(actions);
  }
}

function renderToolReasoningNote(parent, domId, texts, stepIndices = []) {
  const parts = texts.map((t, i) => {
    const idx = stepIndices[i];
    const stored = idx != null ? getToolOutput(idx) : null;
    return stored ? normalizeReasoningText(stored) : normalizeReasoningText(t);
  });
  const full = parts.filter(Boolean).join("\n\n");
  const long = isReasoningLong(full);
  const expanded = ui.expandedReasoningGroups.has(domId);
  const note = el(
    "div",
    "tool-reasoning-note" + (long && !expanded ? " is-collapsed" : " is-expanded"),
  );

  const head = el("div", "tool-reasoning-head");
  head.appendChild(el("span", "tool-reasoning-label", "Reasoning"));
  if (full && normalizeReasoningText(full)) {
    head.appendChild(
      el(
        "span",
        "tool-reasoning-meta",
        `${reasoningLineCount(full)} lines · ${reasoningCharCount(full).toLocaleString()} chars`,
      ),
    );
  }
  if (long) {
    const btn = el(
      "button",
      "tool-reasoning-toggle",
      expanded ? "Collapse" : "Show all",
    );
    btn.type = "button";
    btn.onclick = (e) => {
      e.stopPropagation();
      e.preventDefault();
      if (ui.expandedReasoningGroups.has(domId)) ui.expandedReasoningGroups.delete(domId);
      else ui.expandedReasoningGroups.add(domId);
      const card = parent.closest("[data-block-id]");
      if (card) delete card.dataset.fp;
      scheduleRender(true);
    };
    head.appendChild(btn);
  }
  note.appendChild(head);

  const body = el("div", "tool-reasoning-body");
  fillReasoningBody(body, full, { expanded: !long || expanded });
  note.appendChild(body);
  parent.appendChild(note);
}

function renderReasoningHistoryBlock(parent, block, domId) {
  const full = resolveReasoningFullText(block);
  const expanded = ui.expandedReasoningGroups.has(domId);
  const card = buildReasoningCard(full, {
    expanded,
    onToggle: () => {
      if (ui.expandedReasoningGroups.has(domId)) ui.expandedReasoningGroups.delete(domId);
      else ui.expandedReasoningGroups.add(domId);
      const node = parent.querySelector(`[data-block-id="${domId}"]`) || parent;
      if (node.dataset) delete node.dataset.fp;
      scheduleRender(true);
    },
  });
  parent.appendChild(card);
}

function renderCompactTool(parent, block, meta, blockId) {
  const outHint = toolOutputSummary(block);
  const chip = el("div", `tool-chip status-${block.status} clickable`);
  chip.title = [block.toolName, block.args, outHint].filter(Boolean).join("\n");
  chip.onclick = (e) => {
    e.stopPropagation();
    ui.expandedToolGroups.add(blockId);
    const node = parent.closest("[data-block-id]");
    if (node) delete node.dataset.fp;
    scheduleRender(true);
  };
  chip.appendChild(el("span", "tool-chip-icon tool-glyph", meta.icon));
  const main = el("span", "tool-chip-main");
  main.appendChild(el("span", "tool-chip-name", meta.label));
  appendToolSourceChip(main, meta);
  if (block.args) {
    appendToolArgChips(main, block.args);
  }
  chip.appendChild(main);
  const trail = el("span", "tool-chip-trail");
  if (outHint) trail.appendChild(el("span", "tool-chip-out", outHint));
  if (block.ms) trail.appendChild(el("span", "tool-chip-ms", `${block.ms}ms`));
  const badge = block.status === "ok" ? "✓" : block.status === "err" ? "✗" : "⏳";
  trail.appendChild(el("span", `tool-chip-badge status-${block.status}`, badge));
  chip.appendChild(trail);
  parent.appendChild(chip);
}

function renderToolGroup(parent, block, blockId) {
  const meta = toolMeta(block.toolName);

  if (shouldCompactTool(block, blockId)) {
    renderCompactTool(parent, block, meta, blockId);
    return;
  }

  const card = el("div", `tool-card status-${block.status} is-expanded-view`);
  const header = buildToolCardHeader(meta, block.toolName, block.args, block.status, {
    ms: block.ms,
    collapsible: true,
  });
  header.onclick = () => {
    if (block.status !== "running" && block.status !== "pending") {
      ui.expandedToolGroups.delete(blockId);
      const node = parent.closest("[data-block-id]");
      if (node) delete node.dataset.fp;
      scheduleRender(true);
    }
  };
  card.appendChild(header);

  const body = el("div", "tool-card-body");
  const reasoning = block.steps.filter((s) => s.kind === "reasoning");
  const actionSteps = block.steps.filter((s) => s.kind !== "reasoning" && !(s.kind === "done" && s.output));
  const showTimeline = actionSteps.length > 1;

  if (reasoning.length) {
    renderToolReasoningNote(
      body,
      blockId,
      reasoning.map((s) => s.text),
      reasoning.map((s) => s.index),
    );
  }
  if (showTimeline) {
    const timeline = el("div", "tool-timeline");
    for (const step of actionSteps) {
      const row = el("div", `tool-step kind-${step.kind}`);
      row.appendChild(el("span", "tool-step-icon", stepIcon(step)));
      row.appendChild(el("span", "tool-step-text", formatStepText(step)));
      timeline.appendChild(row);
    }
    body.appendChild(timeline);
  }
  for (const step of block.steps) {
    if (step.kind === "done" && step.output) {
      const outWrap = el("div", "tool-output-wrap");
      if (block.steps.filter((s) => s.output).length > 1) {
        outWrap.appendChild(el("div", "tool-output-label", formatStepText(step)));
      }
      renderToolOutput(outWrap, step.output, step.index, blockId);
      body.appendChild(outWrap);
    }
  }
  if (body.childNodes.length) {
    card.appendChild(body);
  } else {
    card.classList.add("is-compact");
  }

  parent.appendChild(card);
}

function renderChatBubble(parent, block) {
  const row = el("div", `msg-row role-${block.type}`);
  const label = block.type === "you" ? "You" : block.type === "assistant" ? "Assistant" : "Error";
  row.appendChild(el("div", "msg-label", label));
  const bubble = el("div", "msg-bubble");
  if (block.md) {
    bubble.innerHTML = `<div class="md">${renderMarkdown(block.body)}</div>`;
  } else if (block.type === "you") {
    bubble.innerHTML = linkifyPlainText(block.body);
  } else {
    bubble.textContent = block.body;
  }
  row.appendChild(bubble);
  parent.appendChild(row);
}

function renderSystemNote(parent, block) {
  const note = el("div", "msg-system");
  const body = block.body || "";
  if (/^cleared|^new session/i.test(body)) note.classList.add("system-session");
  else if (/approval|denied|approved/i.test(body)) note.classList.add("system-approval");
  note.textContent = body;
  parent.appendChild(note);
}

function appendMessageBlock(parent, block, domId) {
  const wrap = el("div", `msg-block msg-block-${block.type}`);
  if (block.type === "tool-batch") renderToolBatch(wrap, block);
  else if (block.type === "tool-group") renderToolGroup(wrap, block, domId);
  else if (block.type === "reasoning") renderReasoningHistoryBlock(wrap, block, domId);
  else if (block.type === "you" || block.type === "assistant" || block.type === "error") {
    renderChatBubble(wrap, block);
  } else {
    renderSystemNote(wrap, block);
  }
  parent.appendChild(wrap);
  wrap.dataset.blockId = domId;
  wrap.dataset.fp = blockFingerprint(block, domId);
}

function blockDomId(block, i) {
  if (block.type === "tool-batch") return block.batchId || `tb-${i}`;
  if (block.type === "tool-group") return `tg-${block.steps[0]?.index ?? i}`;
  if (block.type === "reasoning") return `rs-${block.index ?? i}`;
  return `${block.type}-${block.index ?? i}`;
}

function blockFingerprint(block, domId) {
  if (block.type === "tool-batch") return toolBatchFingerprint(block);
  if (block.type === "tool-group") {
    const outs = block.steps
      .filter((s) => s.output)
      .map((s) => `${s.index}:${s.output.length}:${s.output.slice(0, 64)}`)
      .join("|");
    const outExpanded = block.steps
      .filter((s) => s.output)
      .map((s) => (ui.expandedToolLines.has(toolOutputKey(domId, s.index)) ? "1" : "0"))
      .join("");
    return JSON.stringify({
      t: "tg",
      id: domId,
      tool: block.toolName,
      status: block.status,
      ms: block.ms,
      args: block.args,
      steps: block.steps.map((s) => `${s.kind}:${s.text}:${s.ok}`),
      outs,
      outExpanded,
      reasoningExp: ui.expandedReasoningGroups.has(domId),
      expanded: ui.expandedToolGroups.has(domId),
    });
  }
  if (block.type === "reasoning") {
    return JSON.stringify({
      t: "rs",
      id: domId,
      body: resolveReasoningFullText(block),
      expanded: ui.expandedReasoningGroups.has(domId),
    });
  }
  return JSON.stringify({ t: block.type, body: block.body });
}

/** Incrementally sync history blocks — only touch changed tail. */
function syncMessageHistory(historyEl, lines) {
  const blocks = mergeConsecutiveToolGroups(buildMessageBlocks(lines));
  const children = [...historyEl.children];

  if (!blocks.length) {
    if (children.length) historyEl.replaceChildren();
    return 0;
  }

  let firstDiff = blocks.length;
  for (let i = 0; i < blocks.length; i++) {
    const domId = blockDomId(blocks[i], i);
    const fp = blockFingerprint(blocks[i], domId);
    const node = children[i];
    if (node?.dataset.blockId === domId && node.dataset.fp === fp) continue;
    firstDiff = i;
    break;
  }

  if (firstDiff === blocks.length && children.length === blocks.length) {
    return blocks.length;
  }

  for (let i = children.length - 1; i >= firstDiff; i--) {
    children[i]?.remove();
  }
  for (let i = firstDiff; i < blocks.length; i++) {
    appendMessageBlock(historyEl, blocks[i], blockDomId(blocks[i], i));
  }
  return blocks.length;
}

/** Live-zone layout only — streaming/reasoning text is patched in place. */
function liveStructureFingerprint() {
  const flow = state.chat_activity_flow;
  return JSON.stringify({
    reasoningCompressing: state.chat_reasoning_compressing,
    activityFlowKind: flow?.kind || null,
    hasActivityFlow: Boolean(flow),
    tool: state.chat_tool_running || state.chat_tool_pending,
    hasReasoning: Boolean(state.chat_reasoning),
    hasStreaming: Boolean(state.chat_streaming),
    thinking: Boolean(
      state.chat_busy &&
        !state.chat_streaming &&
        !state.chat_tool_running &&
        !state.chat_reasoning &&
        !state.chat_reasoning_compressing &&
        !state.chat_activity_flow,
    ),
  });
}

function syncLiveZoneChrome(liveEl) {
  const hasActivity = Boolean(
    state.chat_tool_running ||
      state.chat_tool_pending ||
      state.chat_reasoning ||
      state.chat_reasoning_compressing ||
      state.chat_activity_flow ||
      state.chat_streaming ||
      (state.chat_busy &&
        !state.chat_streaming &&
        !state.chat_tool_running &&
        !state.chat_reasoning &&
        !state.chat_reasoning_compressing &&
        !state.chat_activity_flow),
  );
  liveEl.classList.toggle("has-activity", hasActivity);
}

function patchLiveToolDetail(liveEl) {
  const detail = state.chat_tool_running_detail;
  if (!detail) return false;
  const card = liveEl.querySelector(".live-tool");
  const titleWrap = card?.querySelector(".tool-card-title-wrap");
  if (!titleWrap || titleWrap.querySelector(".tool-arg-chips")) return false;
  titleWrap.querySelector(".tool-card-status-hint")?.remove();
  appendToolArgChips(titleWrap, detail);
  return true;
}

function patchLiveZoneContent(liveEl, activeTool) {
  if (state.chat_streaming && !state.chat_reasoning && !activeTool) {
    const body = liveEl.querySelector(".activity-streaming-body");
    if (!body) return false;
    paintStreamingBody(body, state.chat_streaming);
    return true;
  }
  if (state.chat_reasoning && !state.chat_streaming && !activeTool) {
    const card = liveEl.querySelector(".activity-reasoning");
    const body = card?.querySelector(".activity-reasoning-body");
    if (!body) return false;
    paintReasoningBody(body, state.chat_reasoning, true);
    updateReasoningMeta(card, state.chat_reasoning, { live: true });
    card.classList.remove("collapsed");
    return true;
  }
  if (activeTool && !state.chat_reasoning && !state.chat_streaming) {
    return patchLiveToolDetail(liveEl);
  }
  if (
    state.chat_activity_flow &&
    !state.chat_reasoning &&
    !state.chat_streaming &&
    !activeTool
  ) {
    const body = liveEl.querySelector(".activity-flow-body");
    if (!body) return false;
    const text = state.chat_activity_flow?.text || "";
    if (body.textContent !== text) body.textContent = text;
    return true;
  }
  return false;
}

function buildLiveToolCard(name, detail, pending) {
  const meta = toolMeta(name);
  const card = el("div", "tool-card status-running live-tool is-compact");
  const header = buildToolCardHeader(meta, name, detail, "running", { pending });
  card.appendChild(header);
  return card;
}

function buildLiveReasoningCard(text) {
  return buildReasoningCard(text, {
    live: true,
    expanded: true,
    onToggle: () => {
      ui.liveReasoningExpanded = !ui.liveReasoningExpanded;
      scheduleRender(true);
    },
  });
}

function buildLiveStreamingCard(text) {
  const card = el("div", "activity-streaming");
  const head = el("div", "activity-streaming-head");
  head.appendChild(el("span", "activity-icon", "✦"));
  head.appendChild(el("span", "activity-title", "Assistant"));
  card.appendChild(head);
  const body = el("div", "activity-streaming-body md is-streaming");
  body.innerHTML = streamingPlainHtml(text);
  card.appendChild(body);
  return card;
}

function buildLiveSummarizingCard() {
  const card = el("div", "activity-thinking activity-summarizing");
  card.appendChild(el("span", "tool-spinner", ""));
  card.appendChild(el("span", "activity-title", "Summarizing context…"));
  return card;
}

function buildLiveActivityFlowCard(flow) {
  const card = el("div", "activity-flow");
  const head = el("div", "activity-flow-head");
  const kind = (flow?.kind || "activity").replace(/Skill|Github/, (m) => m.toLowerCase());
  head.appendChild(el("span", "activity-icon", kind === "skill" ? "📚" : "⚡"));
  head.appendChild(el("span", "activity-title", kind === "skill" ? "Skill" : "Activity"));
  card.appendChild(head);
  const body = el("div", "activity-flow-body");
  body.textContent = flow?.text || "";
  card.appendChild(body);
  return card;
}

function buildLiveThinkingCard() {
  const card = el("div", "activity-thinking");
  card.appendChild(el("span", "tool-spinner", ""));
  card.appendChild(el("span", "activity-title", "Thinking…"));
  return card;
}

function syncLiveZone(liveEl) {
  const structFp = liveStructureFingerprint();
  const activeTool = state.chat_tool_running || state.chat_tool_pending;

  if (liveEl.dataset.structFp === structFp) {
    if (patchLiveZoneContent(liveEl, activeTool)) {
      syncLiveZoneChrome(liveEl);
      return;
    }
    syncLiveZoneChrome(liveEl);
    return;
  }

  liveEl.dataset.structFp = structFp;
  delete liveEl.dataset.fp;
  liveEl.replaceChildren();
  const stack = el("div", "activity-stack");

  if (state.chat_tool_running || state.chat_tool_pending) {
    stack.appendChild(
      buildLiveToolCard(
        activeTool,
        state.chat_tool_running_detail,
        Boolean(state.chat_tool_pending && !state.chat_tool_running),
      ),
    );
  }
  if (state.chat_reasoning) {
    stack.appendChild(buildLiveReasoningCard(state.chat_reasoning));
  }
  if (state.chat_reasoning_compressing) {
    stack.appendChild(buildLiveSummarizingCard());
  }
  if (state.chat_activity_flow) {
    stack.appendChild(buildLiveActivityFlowCard(state.chat_activity_flow));
  }
  if (state.chat_streaming) {
    stack.appendChild(buildLiveStreamingCard(state.chat_streaming));
  }
  if (
    state.chat_busy &&
    !state.chat_streaming &&
    !state.chat_tool_running &&
    !state.chat_reasoning &&
    !state.chat_reasoning_compressing &&
    !state.chat_activity_flow
  ) {
    stack.appendChild(buildLiveThinkingCard());
  }

  if (stack.childNodes.length) {
    liveEl.appendChild(stack);
  }
  syncLiveZoneChrome(liveEl);
}

function contextData() {
  return (
    state.chat_context || {
      turn: 0,
      message_tokens: 0,
      tools_tokens: 0,
      tools_body: "",
      tool_names: [],
      skills_tokens: 0,
      skill_blocks: [],
      input_budget: 1,
      context_limit: 1,
      message_count: 0,
      messages: [],
    }
  );
}

function ctxStatsFingerprint(c) {
  const used = (c.message_tokens || 0) + (c.tools_tokens || 0);
  const budget = c.input_budget || 1;
  const limit = c.context_limit || budget;
  const pct = Math.min(100, Math.round((used / budget) * 100));
  return JSON.stringify({
    turn: c.turn,
    message_tokens: c.message_tokens,
    message_count: c.message_count,
    tools_tokens: c.tools_tokens,
    skills_tokens: c.skills_tokens,
    budget,
    limit,
    used,
    pct,
    rev: c.runtime_context_revision,
    tool_names: c.tool_names,
    tools_body: (c.tools_body || "").slice(0, 96),
    skills: (c.skill_blocks || []).map((s) => `${s.name}:${s.tokens}:${(s.body || "").slice(0, 48)}`),
    expandedSkills: [...ui.expandedCtxSkills],
  });
}

function ctxStatsHtml(c) {
  const used = (c.message_tokens || 0) + (c.tools_tokens || 0);
  const budget = c.input_budget || 1;
  const limit = c.context_limit || budget;
  const pct = Math.min(100, Math.round((used / budget) * 100));
  const barCls = pct >= 95 ? "err" : pct >= 80 ? "warn" : "";
  return `
    <div class="ctx-stat-grid">
      <span class="ctx-chip"><span class="ctx-chip-k">Turn</span><strong>${c.turn}</strong></span>
      <span class="ctx-chip"><span class="ctx-chip-k">Msg</span><strong>${formatTokens(c.message_tokens)} · ${c.message_count}</strong></span>
      <span class="ctx-chip"><span class="ctx-chip-k">Tools</span><strong>${formatTokens(c.tools_tokens)}</strong></span>
      <span class="ctx-chip"><span class="ctx-chip-k">Skills</span><strong>${formatTokens(c.skills_tokens)}</strong></span>
    </div>
    <div class="ctx-budget-row">
      <div class="token-bar ctx-budget-bar ${barCls}"><span style="width:${pct}%"></span></div>
      <span class="ctx-budget-label">${formatTokens(used)} / ${formatTokens(budget)} <span class="ctx-budget-of">(${pct}%)</span></span>
    </div>`;
}

function ctxMsgKey(m, i) {
  return `ctx-${i}-${m.role}`;
}

function contextMessagesFingerprint(messages) {
  const msgs = messages || [];
  if (!msgs.length) return "0";
  return msgs
    .map((m, i) => `${i}:${m.role}:${m.tokens}:${m.content?.length ?? 0}:${(m.content || "").slice(0, 48)}`)
    .join("|");
}

function fillCtxBlockContent(contentEl, content, renderMd) {
  const raw = content || "";
  if (!renderMd) {
    contentEl.textContent = raw;
    contentEl.classList.remove("md-rendered");
    delete contentEl.dataset.mdSrc;
    return;
  }
  if (contentEl.dataset.mdSrc === raw && contentEl.classList.contains("md-rendered")) return;
  contentEl.dataset.mdSrc = raw;
  contentEl.classList.add("md-rendered");
  contentEl.innerHTML = renderMarkdown(raw);
}

function ctxMsgFingerprint(m) {
  return `${m.content?.length ?? 0}:${(m.content || "").slice(0, 96)}`;
}

function ctxBlockExpandKey(i) {
  return `ctx-msg-${i}`;
}

function ctxBlockCollapsePolicy(m) {
  const content = m.content || "";
  const lines = content.split("\n").length;
  const role = (m.role || "").toLowerCase();
  if (role.includes("tool")) {
    return { collapsible: content.length > 80 || lines > 2, previewChars: 140 };
  }
  if (role.includes("system")) {
    return { collapsible: content.length > 320 || lines > 6, previewChars: 160 };
  }
  return { collapsible: content.length > 480 || lines > 10, previewChars: 200 };
}

function ctxBlockPreview(content, maxChars) {
  const flat = (content || "").replace(/\s+/g, " ").trim();
  return truncateMiddle(flat, maxChars);
}

function applyCtxBlockExpandState(block, m, i) {
  const { collapsible, previewChars } = ctxBlockCollapsePolicy(m);
  if (!collapsible) {
    block.classList.remove("collapsible", "collapsed");
    const content = block.querySelector(".ctx-content");
    if (content) fillCtxBlockContent(content, m.content, true);
    return;
  }
  const expandKey = ctxBlockExpandKey(i);
  const expanded = ui.expandedCtxBlocks.has(expandKey);
  block.classList.add("collapsible");
  block.classList.toggle("collapsed", !expanded);
  const toggle = block.querySelector(".ctx-block-toggle");
  if (toggle) toggle.textContent = expanded ? "▾" : "▸";
  const preview = block.querySelector(".ctx-preview");
  const content = block.querySelector(".ctx-content");
  if (preview) preview.classList.toggle("hidden", expanded);
  if (content) {
    content.classList.toggle("hidden", !expanded);
    fillCtxBlockContent(content, m.content, expanded);
  }
  if (preview && !expanded) {
    preview.textContent = ctxBlockPreview(m.content, previewChars);
  }
}

function buildCtxBlock(m, i) {
  const block = el("div", `ctx-block ${roleClass(m.role)}`);
  block.dataset.ctxKey = ctxMsgKey(m, i);
  block.dataset.fp = ctxMsgFingerprint(m);
  const { collapsible, previewChars } = ctxBlockCollapsePolicy(m);
  const expandKey = ctxBlockExpandKey(i);
  const expanded = collapsible && ui.expandedCtxBlocks.has(expandKey);

  const head = el("div", "ctx-block-head clickable");
  head.appendChild(el("span", `ctx-role ${roleClass(m.role)}`, m.role));
  head.appendChild(el("span", "ctx-tokens", `${m.tokens} tok`));

  const toggleExpand = () => {
    if (ui.expandedCtxBlocks.has(expandKey)) ui.expandedCtxBlocks.delete(expandKey);
    else ui.expandedCtxBlocks.add(expandKey);
    scheduleRender(true);
  };

  if (collapsible) {
    const toggle = el("button", "ctx-block-toggle", expanded ? "▾" : "▸");
    toggle.type = "button";
    toggle.onclick = (e) => {
      e.stopPropagation();
      toggleExpand();
    };
    head.appendChild(toggle);
    head.onclick = toggleExpand;
  }

  const preview = el("div", "ctx-preview");
  if (collapsible && !expanded) {
    preview.textContent = ctxBlockPreview(m.content, previewChars);
  } else {
    preview.classList.add("hidden");
  }

  const content = el("div", "ctx-content md");
  fillCtxBlockContent(content, m.content, !collapsible || expanded);
  if (collapsible && !expanded) content.classList.add("hidden");

  block.append(head, preview, content);
  if (collapsible) block.classList.toggle("collapsed", !expanded);
  return block;
}

function refreshCtxBlockExpandStates(ctxMsgs, messages) {
  const msgs = messages || [];
  const nodes = [...ctxMsgs.children].filter((n) => n.dataset.ctxKey);
  for (let i = 0; i < msgs.length; i++) {
    if (nodes[i]) applyCtxBlockExpandState(nodes[i], msgs[i], i);
  }
}

function syncContextMessages(ctxMsgs, messages) {
  const msgs = messages || [];
  const existing = [...ctxMsgs.children].filter((n) => n.dataset.ctxKey);
  let firstDiff = msgs.length;
  for (let i = 0; i < msgs.length; i++) {
    const key = ctxMsgKey(msgs[i], i);
    const fp = ctxMsgFingerprint(msgs[i]);
    const node = existing[i];
    if (node?.dataset.ctxKey === key && node.dataset.fp === fp) continue;
    firstDiff = i;
    break;
  }
  if (firstDiff === msgs.length && existing.length === msgs.length) {
    refreshCtxBlockExpandStates(ctxMsgs, msgs);
    return;
  }
  for (let i = existing.length - 1; i >= firstDiff; i--) existing[i]?.remove();
  for (let i = firstDiff; i < msgs.length; i++) {
    ctxMsgs.appendChild(buildCtxBlock(msgs[i], i));
  }
  refreshCtxBlockExpandStates(ctxMsgs, msgs);
  const empty = ctxMsgs.querySelector(".empty");
  if (!msgs.length && !empty) {
    ctxMsgs.appendChild(el("div", "empty", "No messages in context yet"));
  } else if (msgs.length && empty) {
    empty.remove();
  }
}

async function setContextVisible(visible) {
  if (state?.chat_context_visible === visible) return;
  await apiFetch("/api/chat/context", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ visible }),
  });
}

function ensureContextPane(layout) {
  let pane = layout.querySelector(".context-pane");
  if (!state.chat_context_visible) {
    pane?.remove();
    layout.classList.add("no-context");
    return null;
  }
  layout.classList.remove("no-context");
  if (!pane) {
    pane = el("div", "context-pane");
    const header = el("div", "context-header");
    const title = el("span", "context-header-title", "LLM Context");
    header.appendChild(title);
    header.appendChild(el("span", "ctx-rev-badge hidden"));
    const closeBtn = el("button", "ctx-close", "×");
    closeBtn.type = "button";
    closeBtn.title = "Close";
    closeBtn.setAttribute("aria-label", "Close context panel");
    closeBtn.onclick = () => setContextVisible(false);
    header.appendChild(closeBtn);
    pane.appendChild(header);
    pane.appendChild(el("div", "context-stats"));
    pane.appendChild(el("div", "ctx-tools hidden"));
    pane.appendChild(el("div", "ctx-skills"));
    const ctxMsgs = el("div", "context-messages");
    ctxMsgs.onscroll = () => {
      const gap = ctxMsgs.scrollHeight - ctxMsgs.scrollTop - ctxMsgs.clientHeight;
      ui.ctxStickBottom = gap < 80;
    };
    pane.appendChild(ctxMsgs);
    layout.appendChild(pane);
  }
  return pane;
}

function ctxInlineSummary(items, max = 3) {
  if (!items?.length) return "";
  const shown = items.slice(0, max);
  const rest = items.length - shown.length;
  const text = shown.join(", ");
  return rest > 0 ? `${text} +${rest}` : text;
}

function syncContextTools(toolsEl, c) {
  const names = c.tool_names || [];
  const body = c.tools_body || "";
  if (!names.length && !body) {
    toolsEl.classList.add("hidden");
    toolsEl.replaceChildren();
    delete toolsEl.dataset.contentFp;
    delete toolsEl.dataset.ctxToolsBound;
    return;
  }
  toolsEl.classList.remove("hidden");

  const contentFp = JSON.stringify({ names, len: body.length, head: body.slice(0, 160) });
  const chipsExpanded = ui.expandedCtxToolsChips;
  const schemaExpanded = ui.expandedCtxTools;

  if (!toolsEl.dataset.ctxToolsBound || toolsEl.dataset.contentFp !== contentFp) {
    toolsEl.dataset.ctxToolsBound = "1";
    toolsEl.dataset.contentFp = contentFp;
    toolsEl.replaceChildren();

    const head = el("div", "ctx-compact-head");
    const title = el("span", "ctx-section-title", `Tools (${names.length})`);
    const summary = el("span", "ctx-inline-summary", ctxInlineSummary(names, 4));
    summary.title = names.join(", ");
    const chevron = el("span", "ctx-compact-chevron", chipsExpanded ? "▾" : "▸");
    head.append(title, summary, chevron);

    const schemaBtn = el("button", "ctx-tools-toggle", schemaExpanded ? "schema ▾" : "schema");
    schemaBtn.type = "button";
    schemaBtn.onclick = (e) => {
      e.stopPropagation();
      ui.expandedCtxTools = !ui.expandedCtxTools;
      scheduleRender(true);
    };
    head.appendChild(schemaBtn);

    head.onclick = (e) => {
      if (e.target === schemaBtn) return;
      ui.expandedCtxToolsChips = !ui.expandedCtxToolsChips;
      scheduleRender(true);
    };
    toolsEl.appendChild(head);

    const chips = el("div", "ctx-tool-chips");
    for (const name of names) {
      chips.appendChild(el("span", "ctx-tool-chip", name));
    }
    toolsEl.appendChild(chips);

    const pre = el("pre", "ctx-tools-body");
    pre.textContent = body || "(no tool schema text)";
    toolsEl.appendChild(pre);
  } else {
    const pre = toolsEl.querySelector(".ctx-tools-body");
    if (pre && pre.textContent !== body) pre.textContent = body || "(no tool schema text)";
    const title = toolsEl.querySelector(".ctx-section-title");
    if (title) title.textContent = `Tools (${names.length})`;
    const summary = toolsEl.querySelector(".ctx-inline-summary");
    if (summary) {
      summary.textContent = ctxInlineSummary(names, 4);
      summary.title = names.join(", ");
    }
  }

  toolsEl.classList.toggle("chips-expanded", chipsExpanded);
  toolsEl.classList.toggle("is-expanded", schemaExpanded);
  const chevron = toolsEl.querySelector(".ctx-compact-chevron");
  if (chevron) chevron.textContent = chipsExpanded ? "▾" : "▸";
  const toggle = toolsEl.querySelector(".ctx-tools-toggle");
  if (toggle) toggle.textContent = schemaExpanded ? "schema ▾" : "schema";
}

function syncContextSkills(skillsEl, skillBlocks) {
  const blocks = skillBlocks || [];
  const listExpanded = ui.expandedCtxSkillsList;
  const fp = JSON.stringify({
    blocks: blocks.map((s) => `${s.name}:${s.tokens}:${(s.body || "").slice(0, 48)}`),
    expanded: [...ui.expandedCtxSkills],
    listExpanded,
  });
  if (skillsEl.dataset.fp === fp) return;
  skillsEl.dataset.fp = fp;
  skillsEl.replaceChildren();
  if (!blocks.length) {
    skillsEl.classList.add("hidden");
    return;
  }
  skillsEl.classList.remove("hidden");
  skillsEl.classList.toggle("list-expanded", listExpanded);

  const head = el("div", "ctx-compact-head");
  const names = blocks.map((s) => s.name);
  head.append(
    el("span", "ctx-section-title", `Skills (${blocks.length})`),
    el("span", "ctx-inline-summary", ctxInlineSummary(names, 3)),
    el("span", "ctx-compact-chevron", listExpanded ? "▾" : "▸"),
  );
  head.querySelector(".ctx-inline-summary").title = names.join(", ");
  head.onclick = () => {
    ui.expandedCtxSkillsList = !ui.expandedCtxSkillsList;
    scheduleRender(true);
  };
  skillsEl.appendChild(head);

  const list = el("div", "ctx-skills-list");
  for (const sk of blocks) {
    const key = sk.name || String(sk.tokens);
    const expanded = ui.expandedCtxSkills.has(key);
    const chip = el(
      "button",
      "ctx-skill-chip" + (expanded ? " expanded" : ""),
      `${sk.name} · ${sk.tokens}t`,
    );
    chip.type = "button";
    chip.onclick = (e) => {
      e.stopPropagation();
      if (ui.expandedCtxSkills.has(key)) ui.expandedCtxSkills.delete(key);
      else ui.expandedCtxSkills.add(key);
      scheduleRender(true);
    };
    list.appendChild(chip);
    if (expanded && sk.body) {
      const bodyWrap = el("div", "ctx-skill-body-wrap");
      bodyWrap.appendChild(el("pre", "ctx-skill-body", sk.body));
      list.appendChild(bodyWrap);
    }
  }
  skillsEl.appendChild(list);
}

function syncContextPane(layout) {
  const pane = ensureContextPane(layout);
  if (!pane) return null;
  const c = contextData();
  const revBadge = pane.querySelector(".ctx-rev-badge");
  if (revBadge) {
    if (c.runtime_context_revision != null) {
      revBadge.textContent = `rev ${c.runtime_context_revision}`;
      revBadge.classList.remove("hidden");
    } else {
      revBadge.classList.add("hidden");
    }
  }
  const statsFp = ctxStatsFingerprint(c);
  const statsEl = pane.querySelector(".context-stats");
  if (statsEl.dataset.fp !== statsFp) {
    statsEl.dataset.fp = statsFp;
    statsEl.innerHTML = ctxStatsHtml(c);
  }
  syncContextTools(pane.querySelector(".ctx-tools"), c);
  syncContextSkills(pane.querySelector(".ctx-skills"), c.skill_blocks);
  const ctxMsgs = pane.querySelector(".context-messages");
  const ctxRev = state.chat_context_revision;
  const msgsFp = contextMessagesFingerprint(c.messages);
  if (ctxRev !== ui.lastContextRevision || pane.dataset.msgsFp !== msgsFp) {
    if (pane.dataset.msgsFp !== msgsFp) {
      syncContextMessages(ctxMsgs, c.messages);
      pane.dataset.msgsFp = msgsFp;
    }
    ui.lastContextRevision = ctxRev;
  } else {
    refreshCtxBlockExpandStates(ctxMsgs, c.messages);
  }
  if (window.matchMedia("(max-width: 900px)").matches && state.chat_context_visible) {
    pane.classList.add("mobile-open");
  } else {
    pane.classList.remove("mobile-open");
  }
  return ctxMsgs;
}

function scheduleRender(immediate = false) {
  if (immediate) {
    renderQueued = false;
    applyState(state);
    return;
  }
  if (renderQueued) return;
  renderQueued = true;
  requestAnimationFrame(() => {
    renderQueued = false;
    applyState(state);
  });
}

function resetChatUiState() {
  ui.expandedToolLines.clear();
  ui.expandedReasoningGroups.clear();
  ui.expandedToolGroups.clear();
  ui.expandedToolBatches.clear();
  ui.liveReasoningExpanded = false;
  ui.lastHistoryRevision = null;
  ui.lastContextRevision = null;
  ui.lastHistoryLineCount = -1;
  const main = document.getElementById("main");
  const history = main?.querySelector(".msg-history");
  if (history) delete history.dataset.fp;
  const live = main?.querySelector(".live-zone");
  if (live) {
    delete live.dataset.fp;
    delete live.dataset.structFp;
  }
}

function shortSessionId(id) {
  if (!id) return "";
  return String(id).slice(0, 8);
}

function sessionDisplayLabel(sessionId, sessions) {
  if (!sessionId) return "New session";
  const match = (sessions || []).find((s) => s.id === sessionId);
  const title = (match?.title || "").trim();
  if (title) return truncateMiddle(title, 36);
  return `Session ${shortSessionId(sessionId)}`;
}

function bindSessionMenuDismiss() {
  if (ui.sessionMenuBound) return;
  ui.sessionMenuBound = true;
  document.addEventListener("click", (e) => {
    if (!ui.sessionMenuOpen) return;
    if (e.target.closest(".session-picker")) return;
    ui.sessionMenuOpen = false;
    scheduleRender(true);
  });
}

async function fetchSessions() {
  ui.sessionListLoading = true;
  scheduleRender(true);
  const res = await apiFetch("/api/chat/sessions");
  if (res) {
    try {
      ui.sessionList = await res.json();
    } catch {
      ui.sessionList = [];
    }
  }
  ui.sessionListLoading = false;
  scheduleRender(true);
}

async function openSessionMenu() {
  bindSessionMenuDismiss();
  ui.sessionMenuOpen = !ui.sessionMenuOpen;
  if (ui.sessionMenuOpen && !ui.sessionList && !ui.sessionListLoading) {
    await fetchSessions();
    return;
  }
  scheduleRender(true);
}

async function loadChatSession(id) {
  if (!id || state?.chat_busy) return;
  ui.sessionMenuOpen = false;
  const res = await apiFetch(`/api/chat/sessions/${id}`, { method: "POST" });
  if (res) {
    resetChatUiState();
    ui.chatStickBottom = true;
  }
}

async function newChatSession() {
  if (state?.chat_busy) return;
  ui.sessionMenuOpen = false;
  const res = await apiFetch("/api/chat/sessions/new", { method: "POST" });
  if (res) {
    resetChatUiState();
    ui.chatStickBottom = true;
  }
}

function renderSessionMenu(picker, menu) {
  menu.replaceChildren();
  const newBtn = el("button", "session-menu-item session-menu-new", "+ New session");
  newBtn.type = "button";
  newBtn.disabled = Boolean(state.chat_busy);
  newBtn.onclick = (e) => {
    e.stopPropagation();
    newChatSession();
  };
  menu.appendChild(newBtn);

  if (ui.sessionListLoading) {
    menu.appendChild(el("div", "session-menu-status", "Loading sessions…"));
    return;
  }

  const sessions = ui.sessionList || [];
  if (!sessions.length) {
    menu.appendChild(el("div", "session-menu-status", "No saved sessions yet"));
    return;
  }

  const currentId = state.chat_session_id || null;
  for (const sess of sessions) {
    const item = el(
      "button",
      "session-menu-item" + (sess.id === currentId ? " is-active" : ""),
    );
    item.type = "button";
    item.disabled = Boolean(state.chat_busy);
    const title = (sess.title || "").trim() || `Session ${shortSessionId(sess.id)}`;
    item.innerHTML =
      `<span class="session-menu-title">${escapeHtml(truncateMiddle(title, 48))}</span>` +
      `<span class="session-menu-meta">${escapeHtml(sess.created_at)} · ${escapeHtml(shortSessionId(sess.id))}</span>`;
    item.onclick = (e) => {
      e.stopPropagation();
      if (sess.id !== currentId) loadChatSession(sess.id);
      else {
        ui.sessionMenuOpen = false;
        scheduleRender(true);
      }
    };
    menu.appendChild(item);
  }
}

function updateSessionPicker(header) {
  let picker = header.querySelector(".session-picker");
  if (!picker) {
    picker = el("div", "session-picker");
    const count = header.querySelector(".messages-count");
    header.insertBefore(picker, count || header.querySelector(".messages-header-actions"));
  }

  const label = sessionDisplayLabel(state.chat_session_id, ui.sessionList);
  let trigger = picker.querySelector(".session-picker-trigger");
  if (!trigger) {
    trigger = el("button", "session-picker-trigger");
    trigger.type = "button";
    trigger.onclick = (e) => {
      e.stopPropagation();
      openSessionMenu();
    };
    picker.appendChild(trigger);
  }
  trigger.disabled = Boolean(state.chat_busy);
  trigger.innerHTML =
    `<span class="session-picker-label">${escapeHtml(label)}</span>` +
    `<span class="session-picker-chevron" aria-hidden="true">${ui.sessionMenuOpen ? "▴" : "▾"}</span>`;
  if (state.chat_session_id) {
    trigger.title = state.chat_session_id;
  } else {
    trigger.removeAttribute("title");
  }

  let menu = picker.querySelector(".session-menu");
  if (ui.sessionMenuOpen) {
    if (!menu) {
      menu = el("div", "session-menu");
      picker.appendChild(menu);
    }
    renderSessionMenu(picker, menu);
    menu.classList.remove("hidden");
  } else if (menu) {
    menu.classList.add("hidden");
  }
}

function updateMessagesHeader(shell) {
  const header = shell.querySelector(".messages-header");
  if (!header) return;

  updateSessionPicker(header);
  let actions = header.querySelector(".messages-header-actions");
  if (!actions) {
    actions = el("div", "messages-header-actions");
    header.appendChild(actions);
  }

  let clearBtn = actions.querySelector(".btn-header-clear");
  if (!clearBtn) {
    clearBtn = el("button", "btn-header-action btn-header-clear", "Clear");
    clearBtn.type = "button";
    clearBtn.onclick = () => apiFetch("/api/chat/clear", { method: "POST" });
    actions.appendChild(clearBtn);
  }
  const hasHistory = (state.chat_lines || []).length > 0;
  clearBtn.classList.toggle("hidden", !hasHistory || state.chat_busy);

  let ctxBtn = actions.querySelector(".btn-header-ctx");
  if (!ctxBtn) {
    ctxBtn = el("button", "btn-header-action btn-header-ctx", "Context");
    ctxBtn.type = "button";
    ctxBtn.onclick = () => setContextVisible(true);
    actions.appendChild(ctxBtn);
  }
  ctxBtn.classList.toggle("hidden", state.chat_context_visible);

  let live = actions.querySelector(".messages-live");
  if (!live) {
    live = el("span", "messages-live hidden");
    actions.appendChild(live);
  }
  if (state.chat_busy) {
    const meta = phaseMeta(state.chat_turn_phase);
    live.className = `messages-live ${meta.cls}`;
    live.innerHTML = `<span class="live-dot" aria-hidden="true"></span><span>${meta.label}</span>`;
  } else {
    live.className = "messages-live hidden";
    live.replaceChildren();
  }
}

function updateChatInput(shell) {
  const textarea = shell.querySelector(".chat-input textarea");
  const sendBtn = shell.querySelector(".chat-input .btn-primary");
  const cancelBtn = shell.querySelector(".chat-input .btn-cancel");
  if (!textarea) return;
  const pos = textarea.selectionStart;
  if (textarea.value !== ui.chatDraft) textarea.value = ui.chatDraft;
  const ph = state.chat_busy
    ? "Waiting for model…"
    : "Message… (Enter send · Shift+Enter newline · /help · /clear · /new)";
  if (textarea.placeholder !== ph) textarea.placeholder = ph;
  if (textarea.disabled !== state.chat_busy) textarea.disabled = state.chat_busy;
  if (document.activeElement === textarea && pos != null) {
    textarea.selectionStart = pos;
    textarea.selectionEnd = pos;
  }
  if (sendBtn) sendBtn.disabled = state.chat_busy;
  if (cancelBtn) cancelBtn.disabled = !state.chat_busy;
  autoResizeTextarea(textarea);
}

function bindChatShell(shell) {
  if (shell.dataset.bound) return;
  shell.dataset.bound = "1";
  const messages = shell.querySelector(".messages");
  messages.onscroll = () => {
    const gap = messages.scrollHeight - messages.scrollTop - messages.clientHeight;
    ui.chatStickBottom = gap < 80;
    const fab = shell.querySelector(".scroll-fab");
    if (fab) fab.classList.toggle("hidden", ui.chatStickBottom);
  };
  shell.querySelector(".scroll-fab")?.addEventListener("click", () => {
    ui.chatStickBottom = true;
    const m = shell.querySelector(".messages");
    if (m) m.scrollTo({ top: m.scrollHeight, behavior: "smooth" });
  });
  const textarea = shell.querySelector(".chat-input textarea");
  textarea.oninput = () => {
    ui.chatDraft = textarea.value;
    autoResizeTextarea(textarea);
  };
  textarea.onkeydown = (e) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      shell.querySelector(".chat-input .btn-primary")?.click();
    }
  };
  shell.querySelector(".chat-input .btn-primary")?.addEventListener("click", async () => {
    const msg = textarea.value.trim();
    if (!msg) return;
    textarea.value = "";
    ui.chatDraft = "";
    ui.chatStickBottom = true;
    await apiFetch("/api/chat", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ message: msg }),
    });
  });
  shell.querySelector(".chat-input .btn-cancel")?.addEventListener("click", () => {
    apiFetch("/api/chat/cancel", { method: "POST" });
  });
}

function buildChatShell() {
  const shell = el("div", "chat-shell");
  const layout = el("div", "chat-layout");
  const messagesPane = el("div", "messages-pane");
  const header = el("div", "messages-header");
  header.appendChild(el("span", "messages-title", "Messages"));
  header.appendChild(el("span", "messages-count", ""));
  header.appendChild(el("div", "messages-header-actions"));
  messagesPane.appendChild(header);
  const messages = el("div", "messages");
  messages.appendChild(el("div", "msg-history"));
  const divider = el("div", "live-divider hidden");
  divider.innerHTML =
    '<span class="live-divider-line"></span><span class="live-divider-text">In progress</span><span class="live-divider-line"></span>';
  messages.appendChild(divider);
  messages.appendChild(el("div", "live-zone"));
  messagesPane.appendChild(messages);
  const fab = el("button", "scroll-fab hidden", "↓ Bottom");
  messagesPane.appendChild(fab);
  const ctxFab = el("button", "ctx-fab hidden", "Context");
  ctxFab.onclick = () => setContextVisible(!state.chat_context_visible);
  messagesPane.appendChild(ctxFab);
  layout.appendChild(messagesPane);
  shell.appendChild(layout);
  const inputRow = el("div", "chat-input");
  const textarea = document.createElement("textarea");
  textarea.rows = 1;
  const sendBtn = el("button", "btn btn-primary", "Send");
  const cancelBtn = el("button", "btn btn-ghost btn-cancel", "Cancel");
  inputRow.append(textarea, sendBtn, cancelBtn);
  shell.appendChild(inputRow);
  bindChatShell(shell);
  return shell;
}

function ciPill(summary) {
  const s = (summary || "").toLowerCase();
  if (s.includes("fail") || s.includes("red")) return '<span class="pill err">fail</span>';
  if (s.includes("ok") || s.includes("pass") || s.includes("green")) return '<span class="pill ok">ok</span>';
  if (s.includes("pending") || s.includes("wait")) return '<span class="pill warn">wait</span>';
  return '<span class="pill muted">—</span>';
}

function updateTabs() {
  const nav = document.getElementById("tabs");
  if (!state) return;
  const approvalCount = (state.approvals || []).length;
  const tabFp = JSON.stringify({ tabs: state.tabs, active: state.tab, approvals: approvalCount });
  if (nav.dataset.fp === tabFp) return;
  nav.dataset.fp = tabFp;

  const want = new Set(state.tabs);
  for (const child of [...nav.children]) {
    if (!want.has(child.dataset.tab)) child.remove();
  }
  for (const tab of state.tabs) {
    let btn = nav.querySelector(`[data-tab="${tab}"]`);
    if (!btn) {
      const label = `${TAB_ICONS[tab] || ""} ${TAB_LABELS[tab] || tab}`.trim();
      btn = el("button", "tab", label);
      btn.dataset.tab = tab;
      btn.onclick = () => setTab(tab);
      nav.appendChild(btn);
    }
    btn.className = "tab" + (state.tab === tab ? " active" : "");
    let badge = btn.querySelector(".tab-badge");
    if (tab === "approvals" && approvalCount > 0) {
      if (!badge) {
        badge = el("span", "tab-badge");
        btn.appendChild(badge);
      }
      badge.textContent = String(approvalCount);
    } else if (badge) {
      badge.remove();
    }
  }
}

async function setTab(tab) {
  await apiFetch(`/api/tab/${tab}`, { method: "POST" });
}

function updateStatus() {
  const dot = document.getElementById("conn-dot");
  const s = document.getElementById("status");
  const footer = document.getElementById("footer");
  const ctxEl = document.getElementById("ctx-usage");
  if (!state) return;

  const live = ws && ws.readyState === WebSocket.OPEN ? "live" : "dead";
  if (dot.className !== `brand-dot ${live}`) dot.className = `brand-dot ${live}`;

  const parts = [state.status || "ready"];
  if (state.engine_busy) parts.push(state.engine_workflow_id || "workflow");
  if (state.chat_busy) {
    const phase = state.chat_turn_phase;
    if (phase) parts.push(phase);
    else parts.push("chat");
  }
  if (ui.statusError) parts.push(ui.statusError);
  const statusText = parts.join(" · ");
  s.title = statusText;
  s.classList.toggle("is-error", Boolean(ui.statusError));
  if (s.textContent !== statusText) s.textContent = statusText;

  if (state.tab === "chat" && state.chat_context_visible) {
    const c = contextData();
    const used = (c.message_tokens || 0) + (c.tools_tokens || 0);
    const budget = c.input_budget || 1;
    const limit = c.context_limit || budget;
    const pct = Math.min(100, Math.round((used / budget) * 100));
    const cls = pct >= 95 ? "err" : pct >= 80 ? "warn" : "";
    const ctxFp = `${used}:${budget}:${limit}:${pct}`;
    ctxEl.classList.remove("hidden");
    if (ctxEl.dataset.fp !== ctxFp) {
      ctxEl.dataset.fp = ctxFp;
      ctxEl.innerHTML = `
        <div>ctx ${formatTokens(used)} / ${formatTokens(budget)} of ${formatTokens(limit)} (${pct}%)</div>
        <div class="token-bar ${cls}"><span style="width:${pct}%"></span></div>`;
    }
  } else if (!ctxEl.classList.contains("hidden")) {
    ctxEl.classList.add("hidden");
    delete ctxEl.dataset.fp;
  }

  const footerText = state.attach_mode
    ? "attach mode · shared store with daemon"
    : `model: ${state.llm_model} · repos: ${(state.repos || []).join(", ") || "—"}`;
  if (footer.textContent !== footerText) footer.textContent = footerText;
}

function formatTokens(n) {
  if (n >= 10000) return (n / 1000).toFixed(1) + "k";
  if (n >= 1000) return (n / 1000).toFixed(2) + "k";
  return String(n);
}

function formatShortTime(ts) {
  if (ts == null || ts === "") return "";
  const d =
    typeof ts === "number"
      ? new Date(ts < 1e12 ? ts * 1000 : ts)
      : new Date(ts);
  if (Number.isNaN(d.getTime())) return String(ts);
  return d.toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
  });
}

function ensureLiveDivider(messages, live) {
  let divider = messages.querySelector(".live-divider");
  if (!divider) {
    divider = el("div", "live-divider hidden");
    divider.innerHTML =
      '<span class="live-divider-line"></span><span class="live-divider-text">In progress</span><span class="live-divider-line"></span>';
    messages.insertBefore(divider, live);
  }
  const show = Boolean(state?.chat_busy) || live.classList.contains("has-activity");
  divider.classList.toggle("hidden", !show);
}

function ensureMsgStructure(messages) {
  let history = messages.querySelector(".msg-history");
  let live = messages.querySelector(".live-zone");
  if (!history || !live) {
    const nodes = [...messages.childNodes];
    messages.replaceChildren();
    history = el("div", "msg-history");
    const divider = el("div", "live-divider hidden");
    divider.innerHTML =
      '<span class="live-divider-line"></span><span class="live-divider-text">In progress</span><span class="live-divider-line"></span>';
    live = el("div", "live-zone");
    for (const n of nodes) {
      if (n.classList?.contains("empty")) history.appendChild(n);
      else if (!n.classList?.contains("live-card")) history.appendChild(n);
    }
    messages.append(history, divider, live);
  }
  return { history, live };
}

function updateChat(main, mode = "full") {
  let shell = main.querySelector(".chat-shell");
  if (!shell) {
    main.replaceChildren();
    shell = buildChatShell();
    main.appendChild(shell);
  }

  const layout = shell.querySelector(".chat-layout");
  const messages = shell.querySelector(".messages");
  const { history, live } = ensureMsgStructure(messages);
  const countEl = shell.querySelector(".messages-count");
  const fab = shell.querySelector(".scroll-fab");

  const prevBottom = ui.chatStickBottom;

  if (mode !== "live") {
    const lines = state.chat_lines || [];
    const empty = history.querySelector(".empty");
    const histRev = state.chat_history_revision;

    if (!lines.length && !state.chat_busy) {
      if (!empty) {
        history.replaceChildren();
        history.appendChild(el("div", "empty empty-chat", "Send a message to start coding…"));
      }
      if (countEl) countEl.textContent = "";
      ui.lastHistoryRevision = histRev;
      ui.lastHistoryLineCount = 0;
    } else if (
      histRev !== ui.lastHistoryRevision ||
      mode === "full" ||
      lines.length !== ui.lastHistoryLineCount
    ) {
      if (empty) empty.remove();
      const blockCount = syncMessageHistory(history, lines);
      const stats = messageStatsFromLines(lines);
      if (countEl) {
        countEl.textContent = formatMessageCount(stats) || (lines.length ? `${blockCount} blocks` : "");
      }
      ui.lastHistoryRevision = histRev;
      ui.lastHistoryLineCount = lines.length;
    }
  }

  ensureLiveDivider(messages, live);
  syncLiveZone(live);
  updateMessagesHeader(shell);

  let ctxMsgs = null;
  if (mode !== "live") {
    ctxMsgs = syncContextPane(layout);
  } else {
    ensureContextPane(layout);
    if (state.chat_context_visible) {
      const pane = layout.querySelector(".context-pane");
      if (pane) {
        const narrow = window.matchMedia("(max-width: 900px)").matches;
        pane.classList.toggle("mobile-open", narrow);
      }
    }
  }
  updateChatInput(shell);
  if (fab) fab.classList.toggle("hidden", ui.chatStickBottom);
  const ctxFab = shell.querySelector(".ctx-fab");
  if (ctxFab) {
    const narrow = window.matchMedia("(max-width: 900px)").matches;
    ctxFab.classList.toggle("hidden", !narrow);
    ctxFab.textContent = state.chat_context_visible ? "Close" : "Context";
    ctxFab.setAttribute("aria-label", state.chat_context_visible ? "Close context panel" : "Show context panel");
  }
  if (ctxMsgs && ui.ctxStickBottom) {
    ctxMsgs.scrollTop = ctxMsgs.scrollHeight;
  }

  if (prevBottom) {
    messages.scrollTop = messages.scrollHeight;
  }
}

function applyChatLiveRender() {
  if (!state || state.tab !== "chat") {
    updateStatus();
    return;
  }
  const main = document.getElementById("main");
  updateChat(main, "live");
  updateApprovalModal();
}

function applyChatStructuralRender() {
  if (!state || state.tab !== "chat") {
    updateStatus();
    updateApprovalModal();
    return;
  }
  if (prevState?.chat_busy && !state.chat_busy) {
    ui.liveReasoningExpanded = false;
  }
  updateStatus();
  const main = document.getElementById("main");
  updateChat(main, "structural");
  updateApprovalModal();
  prevState = state;
}

function renderDashboard(main) {
  const split = el("div", "split-panel");
  const listPane = el("div", "split-list");
  const toolbar = el("div", "toolbar");
  toolbar.appendChild(el("button", "btn btn-ghost", "Run daily-work"));
  toolbar.lastChild.onclick = () => apiFetch("/api/workflows/daily-work", { method: "POST" });
  toolbar.appendChild(el("button", "btn btn-ghost", "Run review-radar"));
  toolbar.lastChild.onclick = () => apiFetch("/api/workflows/review-radar", { method: "POST" });
  listPane.appendChild(toolbar);

  const list = el("ul", "list");
  const history = state.digest_history || [];
  if (!history.length) {
    listPane.appendChild(el("div", "empty", "No digests yet"));
  } else {
    const activeDate = ui.selectedDigestDate || state.selected_digest_date || history[0]?.date;
    history.forEach((d, index) => {
      const li = el("li", "list-item" + (d.date === activeDate ? " selected" : ""));
      const attn = d.needs_attention > 0 ? `<span class="pill warn">${d.needs_attention} attn</span>` : "";
      const gates = d.policy_gates ? ` · ${escapeHtml(d.policy_gates)}` : "";
      const duration = d.duration_label ? ` · ${escapeHtml(d.duration_label)}` : "";
      li.innerHTML = `<div class="list-item-title">${d.date}</div>
        <div class="list-item-meta">${attn}${d.complete ? "" : '<span class="pill warn">updating</span>'} ign ${d.ignorable} · flaky ${d.flaky_candidates}${gates}${duration}</div>`;
      li.onclick = () => apiFetch(`/api/digest/${index}/select`, { method: "POST" });
      list.appendChild(li);
    });
    listPane.appendChild(list);
  }

  const detail = el("div", "split-detail");
  const date = ui.selectedDigestDate || state.selected_digest_date || history[0]?.date;
  if (date && state.digest_bodies?.[date]) {
    const md = el("div", "md");
    md.innerHTML = renderMarkdown(state.digest_bodies[date]);
    detail.appendChild(md);
  } else {
    detail.appendChild(el("div", "empty", "Select a digest"));
  }

  split.append(listPane, detail);
  main.appendChild(split);
}

function renderPrs(main) {
  const split = el("div", "split-panel");
  const listPane = el("div", "split-list");
  const prs = state.prs || [];
  const selectedIdx = state.selected_pr_index ?? ui.selectedPrIndex ?? 0;

  const toolbar = el("div", "toolbar");
  const filterBtn = el("button", "btn btn-ghost", `Filter: ${state.pr_filter || "all"}`);
  filterBtn.onclick = () => apiFetch("/api/prs/filter", { method: "POST" });
  const sortBtn = el("button", "btn btn-ghost", `Sort: ${state.pr_sort || "default"}`);
  sortBtn.onclick = () => apiFetch("/api/prs/sort", { method: "POST" });
  const triageBtn = el("button", "btn btn-ghost", "Triage");
  triageBtn.disabled = !prs.length;
  triageBtn.onclick = () => apiFetch(`/api/prs/${selectedIdx}/triage`, { method: "POST" });
  toolbar.append(filterBtn, sortBtn, triageBtn);
  listPane.appendChild(toolbar);

  const list = el("ul", "list");
  if (!prs.length) {
    listPane.appendChild(el("div", "empty", "No PRs in store"));
  } else {
    prs.forEach((p, i) => {
      const li = el("li", "list-item" + (i === selectedIdx ? " selected" : ""));
      const triageMark = p.triage_note ? ' <span class="triage-mark" title="triage">◆</span>' : "";
      const reviewMeta = p.review_summary ? ` · ${escapeHtml(p.review_summary)}` : "";
      li.innerHTML = `<div class="list-item-title">${ciPill(p.ci_summary)}#${p.number} ${escapeHtml(p.title)}${triageMark}</div>
        <div class="list-item-meta">${escapeHtml(p.repo)} · ${escapeHtml(p.author)}${p.is_draft ? " · draft" : ""}${reviewMeta}</div>`;
      li.onclick = async () => {
        await apiFetch(`/api/prs/${i}/select`, { method: "POST" });
        await apiFetch(`/api/prs/${i}/overview`, { method: "POST" });
      };
      list.appendChild(li);
    });
    listPane.appendChild(list);
  }

  const detail = el("div", "split-detail");
  if (state.pr_overview_loading) {
    const loading = el("div", "overview-loading");
    loading.appendChild(el("div", "spinner"));
    loading.appendChild(el("span", "", "Loading overview…"));
    detail.appendChild(loading);
  } else if (state.pr_overview) {
    const md = el("div", "md");
    md.innerHTML = renderMarkdown(state.pr_overview);
    detail.appendChild(md);
  } else if (prs.length) {
    detail.appendChild(el("div", "empty", "Select a PR to load overview"));
  }

  split.append(listPane, detail);
  main.appendChild(split);
}

function renderApprovals(main) {
  const panel = el("div", "panel");
  const approvals = state.approvals || [];
  if (!approvals.length) {
    panel.appendChild(el("div", "empty", "No pending approvals"));
  } else {
    for (const a of approvals) {
      const parsed = parseApprovalDescription(a.description, a.kind?.replace(/_/g, " "));
      const card = el("div", "approval-card" + (parsed.verdict === "REJECT" ? " verdict-reject" : ""));
      const header = el("div", "approval-card-header");
      const metaParts = [];
      if (a.repo) metaParts.push(escapeHtml(a.repo));
      if (a.pr_number != null) metaParts.push(`#${a.pr_number}`);
      if (a.status) metaParts.push(escapeHtml(a.status));
      header.innerHTML = `<h4>${escapeHtml(a.kind.replace(/_/g, " "))}</h4>${
        metaParts.length ? `<div class="approval-card-meta">${metaParts.join(" · ")}</div>` : ""
      }`;
      card.appendChild(header);
      card.appendChild(buildApprovalDescription(parsed));
      const actions = el("div", "approval-actions");
      const row = el("div", "approval-btn-row");
      const no = el("button", `btn ${parsed.verdict === "REJECT" ? "btn-primary" : "btn-danger"}`, "Deny");
      const ok = el("button", `btn ${parsed.verdict === "REJECT" ? "btn-warn" : "btn-primary"}`, parsed.verdict === "REJECT" ? "Approve anyway" : "Approve");
      ok.onclick = () => decide(a.id, true);
      no.onclick = () => decide(a.id, false);
      row.append(no, ok);
      actions.appendChild(row);
      card.appendChild(actions);
      panel.appendChild(card);
    }
  }
  main.appendChild(panel);
}

async function decide(id, approve) {
  await apiFetch(`/api/approvals/${id}`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ approve }),
  });
}

function renderLogs(main) {
  const panel = el("div", "panel log-list");
  const toolbar = el("div", "toolbar");
  const filterBtn = el("button", "btn btn-ghost", `Filter: ${state.log_filter || "all"}`);
  filterBtn.onclick = () => apiFetch("/api/logs/filter", { method: "POST" });
  toolbar.appendChild(filterBtn);
  panel.appendChild(toolbar);

  const logs = state.logs || [];
  if (!logs.length) {
    panel.appendChild(el("div", "empty", "No logs"));
  } else {
    for (const l of logs) {
      const row = el("div", "log-row");
      const level = (l.level || "info").toLowerCase();
      row.appendChild(el("div", `log-level log-pill ${level}`, level));
      const msgCol = el("div", "log-msg-col");
      const ts = formatShortTime(l.ts);
      if (ts) msgCol.appendChild(el("span", "log-ts", ts));
      msgCol.appendChild(el("span", "log-msg", l.message));
      row.appendChild(msgCol);
      panel.appendChild(row);
    }
  }
  main.appendChild(panel);
}

function renderConfig(main) {
  const grid = el("div", "config-grid");

  const paths = el("div", "card");
  paths.innerHTML = `<h3>Paths</h3><dl>
    <dt>Config</dt><dd>${escapeHtml(state.config_path)}</dd>
    <dt>Repos</dt><dd>${escapeHtml((state.repos || []).join(", ") || "—")}</dd>
  </dl>`;
  grid.appendChild(paths);

  const llm = el("div", "card");
  llm.innerHTML = `<h3>LLM</h3><dl>
    <dt>Model</dt><dd>${escapeHtml(state.llm_model)}</dd>
    <dt>Probe</dt><dd class="${state.llm_ok ? "status-ok" : "status-err"}">${state.llm_ok ? "ok" : "fail"}${state.llm_latency_ms != null ? ` (${state.llm_latency_ms}ms)` : ""}</dd>
  </dl>`;
  grid.appendChild(llm);

  const gh = el("div", "card");
  gh.innerHTML = `<h3>GitHub</h3><dl>
    <dt>Probe</dt><dd class="${state.github_ok ? "status-ok" : "status-err"}">${state.github_ok ? "ok" : "fail"}${state.github_latency_ms != null ? ` (${state.github_latency_ms}ms)` : ""}</dd>
  </dl>`;
  grid.appendChild(gh);

  const mcpCard = el("div", "card");
  const mcpRows = (state.mcp_servers || [])
    .map((s) => {
      const status = s.connected
        ? `ok (${s.tool_count || 0} tools${s.last_rpc_ms != null ? `, ${s.last_rpc_ms}ms` : ""})`
        : s.last_error
          ? `err (${escapeHtml(s.last_error)})`
          : "offline";
      const cls = s.connected ? "status-ok" : s.last_error ? "status-err" : "";
      return `<dt>mcp[${escapeHtml(s.id)}]</dt><dd class="${cls}">${escapeHtml(status)}</dd>`;
    })
    .join("");
  mcpCard.innerHTML = `<h3>MCP</h3><dl>${mcpRows || "<dt>—</dt><dd>no servers configured</dd>"}</dl>`;
  grid.appendChild(mcpCard);

  const actions = el("div", "card");
  actions.innerHTML = "<h3>Actions</h3>";
  const refresh = el("button", "btn btn-ghost", "Refresh store");
  refresh.onclick = () => apiFetch("/api/store/refresh", { method: "POST" });
  const probe = el("button", "btn btn-ghost", "Re-probe");
  probe.onclick = () => apiFetch("/api/config/probe", { method: "POST" });
  actions.append(refresh, probe);
  grid.appendChild(actions);

  main.appendChild(grid);
}

function clearApprovalArmTimer() {
  if (ui.approvalArmTimer != null) {
    clearTimeout(ui.approvalArmTimer);
    ui.approvalArmTimer = null;
  }
}

function approvalIsArmed(d) {
  return Boolean(d.approve_armed || Date.now() >= ui.approvalArmAt);
}

function scheduleApprovalArmRefresh() {
  clearApprovalArmTimer();
  const wait = Math.max(0, ui.approvalArmAt - Date.now());
  if (wait === 0) return;
  ui.approvalArmTimer = setTimeout(() => {
    ui.approvalArmTimer = null;
    scheduleRender(true);
  }, wait + 5);
}

function syncApprovalArmDeadline(d) {
  if (d.id !== ui.approvalDialogId) {
    ui.approvalDialogId = d.id;
    const ms = Number(d.approve_arm_ms_remaining) || 0;
    ui.approvalArmAt = Date.now() + ms;
    scheduleApprovalArmRefresh();
    return;
  }
  if (!approvalIsArmed(d) && ui.approvalArmTimer == null) {
    scheduleApprovalArmRefresh();
  }
}

function parseApprovalDescription(description, toolName) {
  const raw = (description || "").trim();
  let verdict = null;
  let source = "human";
  let issues = [];
  let summary = raw;

  const m = raw.match(/^Chat:\s*([\w.-]+)\s*[—–-]\s*LLM safety review\s+(APPROVE|REJECT)\s*\(([\s\S]+)\)\s*$/i);
  if (m) {
    source = "llm-review";
    toolName = toolName || m[1];
    verdict = m[2].toUpperCase();
    const body = m[3].trim();
    issues = body.split(/\s*;\s*/).map((s) => s.trim()).filter(Boolean);
    if (issues.length <= 1 && body.length > 100) {
      issues = body.split(/(?<=[。；!！?？])\s+/).map((s) => s.trim()).filter(Boolean);
    }
    summary = issues[0] || body;
  } else if (/\bREJECT\b/i.test(raw)) {
    verdict = "REJECT";
    source = "llm-review";
  } else if (/\bAPPROVE\b/i.test(raw)) {
    verdict = "APPROVE";
    source = "llm-review";
  }

  return { raw, source, verdict, summary, issues, toolName: toolName || "tool" };
}

function buildApprovalDescription(parsed) {
  const wrap = el("div", "approval-detail");
  if (parsed.source === "llm-review") {
    const banner = el("div", `approval-verdict-banner verdict-${(parsed.verdict || "unknown").toLowerCase()}`);
    const icon = parsed.verdict === "REJECT" ? "⛔" : parsed.verdict === "APPROVE" ? "✓" : "⚠";
    banner.appendChild(el("span", "approval-verdict-icon", icon));
    const text = el("div", "approval-verdict-text");
    text.appendChild(el("strong", "", `LLM safety review · ${parsed.verdict || "REVIEW"}`));
    text.appendChild(el("span", "", parsed.verdict === "REJECT"
      ? "Automated review flagged risks — read before approving."
      : "Review passed — confirm to proceed."));
    banner.appendChild(text);
    wrap.appendChild(banner);
  }

  if (parsed.issues.length > 1) {
    const list = el("ul", "approval-issues");
    for (const issue of parsed.issues) {
      list.appendChild(el("li", "", issue));
    }
    wrap.appendChild(list);
  } else if (parsed.summary) {
    const body = el("div", "approval-summary", parsed.summary);
    wrap.appendChild(body);
  }

  if (parsed.raw.length > 280 && parsed.issues.length <= 1) {
    const more = el("details", "approval-more");
    more.appendChild(el("summary", "", "Full review text"));
    more.appendChild(el("pre", "approval-raw", parsed.raw));
    wrap.appendChild(more);
  }

  return wrap;
}

function buildApprovalActions(d, armed, deciding, parsed) {
  const actions = el("div", "approval-actions");
  if (deciding) {
    actions.appendChild(el("div", "approval-wait", "Sending decision…"));
    return actions;
  }

  const rejectRecommended = parsed.verdict === "REJECT";
  const hint = el("div", "approval-hint", rejectRecommended
    ? "Deny is recommended when safety review rejected the action."
    : "Mutating action — approve only if you trust this operation.");
  actions.appendChild(hint);

  const row = el("div", "approval-btn-row");
  const no = el("button", `btn ${rejectRecommended ? "btn-primary" : "btn-danger"}`, rejectRecommended ? "Deny (recommended)" : "Deny");
  no.onclick = () => decide(d.id, false);

  const msLeft = Math.max(0, ui.approvalArmAt - Date.now());
  const okLabel = armed
    ? (rejectRecommended ? "Approve anyway" : "Approve")
    : `Approve (${Math.max(1, Math.ceil(msLeft / 50) * 50)}ms)`;
  const ok = el("button", `btn ${rejectRecommended ? "btn-warn" : "btn-primary"}`, okLabel);
  ok.disabled = !armed;
  ok.onclick = () => decide(d.id, true);

  row.append(no, ok);
  actions.appendChild(row);
  return actions;
}

function buildApprovalBox(d, armed, deciding) {
  const parsed = parseApprovalDescription(d.description, d.tool_name);
  const rejectRecommended = parsed.verdict === "REJECT";
  const box = el("div", "approval-box" + (rejectRecommended ? " verdict-reject" : ""));

  const head = el("div", "approval-head");
  head.appendChild(el("div", "approval-head-icon", "⚠"));
  const titles = el("div", "approval-head-text");
  titles.appendChild(el("h3", "", deciding ? "Processing…" : "Approval required"));
  titles.appendChild(el("div", "approval-subtitle", "Mutating tool needs your confirmation"));
  head.appendChild(titles);
  box.appendChild(head);

  const toolRow = el("div", "approval-tool-row");
  toolRow.appendChild(el("span", "approval-tool-label", "Tool"));
  toolRow.appendChild(el("code", "approval-tool-name", parsed.toolName));
  box.appendChild(toolRow);

  box.appendChild(buildApprovalDescription(parsed));
  box.appendChild(buildApprovalActions(d, armed, deciding, parsed));
  return box;
}

function patchApprovalArmButtons(box, armed, parsed) {
  const row = box.querySelector(".approval-btn-row");
  if (!row || row.children.length < 2) return false;
  const no = row.children[0];
  const ok = row.children[1];
  const rejectRecommended = parsed.verdict === "REJECT";
  const msLeft = Math.max(0, ui.approvalArmAt - Date.now());
  const okLabel = armed
    ? (rejectRecommended ? "Approve anyway" : "Approve")
    : `Approve (${Math.max(1, Math.ceil(msLeft / 50) * 50)}ms)`;
  ok.textContent = okLabel;
  ok.disabled = !armed;
  if (no && rejectRecommended) no.textContent = "Deny (recommended)";
  return true;
}

function updateApprovalModal() {
  const d = state?.approval_dialog;
  if (!d) {
    clearApprovalArmTimer();
    ui.approvalDialogId = null;
    ui.approvalArmAt = 0;
    document.querySelectorAll(".approval-modal").forEach((n) => n.remove());
    document.removeEventListener("keydown", onApprovalKeydown);
    return;
  }

  syncApprovalArmDeadline(d);
  const armed = approvalIsArmed(d);
  const deciding = Boolean(d.deciding);
  const parsed = parseApprovalDescription(d.description, d.tool_name);
  const stableFp = JSON.stringify({
    id: d.id,
    deciding,
    tool: d.tool_name,
    desc: d.description,
    verdict: parsed.verdict,
  });
  const armFp = JSON.stringify({ armed, ms: Math.max(0, ui.approvalArmAt - Date.now()) });

  let modal = document.querySelector(".approval-modal");
  if (modal?.dataset.stableFp === stableFp && !deciding) {
    const box = modal.querySelector(".approval-box");
    if (box && modal.dataset.armFp !== armFp) {
      patchApprovalArmButtons(box, armed, parsed);
      modal.dataset.armFp = armFp;
    }
    return;
  }

  const fp = stableFp + armFp;
  if (modal?.dataset.fp === fp) return;

  if (!modal) {
    modal = el("div", "approval-modal");
    modal.onclick = (e) => {
      if (e.target === modal && !state.approval_dialog?.deciding) {
        decide(state.approval_dialog.id, false);
      }
    };
    document.body.appendChild(modal);
    document.addEventListener("keydown", onApprovalKeydown);
  }
  modal.dataset.fp = fp;
  modal.dataset.stableFp = stableFp;
  modal.dataset.armFp = armFp;
  modal.replaceChildren(buildApprovalBox(d, armed, deciding));
}

function onApprovalKeydown(e) {
  if (!state?.approval_dialog || state.approval_dialog.deciding) return;
  const d = state.approval_dialog;
  if (e.key === "Escape") {
    e.preventDefault();
    decide(d.id, false);
    return;
  }
  if (e.key === "y" || e.key === "Y") {
    if (!approvalIsArmed(d)) return;
    e.preventDefault();
    decide(d.id, true);
    return;
  }
  if (e.key === "Enter" && !e.shiftKey) {
    if (!approvalIsArmed(d)) return;
    e.preventDefault();
    decide(d.id, true);
  }
}

function syncSelectionFromState() {
  if (!state) return;
  if (state.selected_digest_date) ui.selectedDigestDate = state.selected_digest_date;
  if (state.selected_pr_index != null) ui.selectedPrIndex = state.selected_pr_index;
}

function mainViewFingerprint() {
  switch (state.tab) {
    case "dashboard":
      return JSON.stringify({
        h: state.digest_history,
        d: ui.selectedDigestDate || state.selected_digest_date,
        bodies: state.digest_bodies,
      });
    case "prs":
      return JSON.stringify({
        prs: state.prs,
        i: state.selected_pr_index ?? ui.selectedPrIndex,
        overview: state.pr_overview,
        loading: state.pr_overview_loading,
        filter: state.pr_filter,
        sort: state.pr_sort,
      });
    case "approvals":
      return JSON.stringify(state.approvals);
    case "logs":
      return JSON.stringify({ logs: state.logs, filter: state.log_filter });
    case "config":
      return JSON.stringify({
        path: state.config_path,
        repos: state.repos,
        llm: state.llm_model,
        llm_ok: state.llm_ok,
        gh_ok: state.github_ok,
      });
    default:
      return state.tab;
  }
}

function renderMainPanel(main, force = false) {
  const fp = mainViewFingerprint();
  if (!force && main.dataset.viewFp === fp && main.dataset.viewTab === state.tab) return;
  main.dataset.viewFp = fp;
  main.dataset.viewTab = state.tab;
  main.replaceChildren();
  switch (state.tab) {
    case "dashboard": renderDashboard(main); break;
    case "prs": renderPrs(main); break;
    case "approvals": renderApprovals(main); break;
    case "logs": renderLogs(main); break;
    case "config": renderConfig(main); break;
    default: main.appendChild(el("div", "empty", "Unknown tab"));
  }
}

function applyState() {
  if (!state) return;
  if (prevState?.chat_busy && !state.chat_busy) {
    ui.liveReasoningExpanded = false;
  }
  const tabChanged = prevState?.tab !== state.tab;
  updateTabs();
  updateStatus();
  syncSelectionFromState();
  const main = document.getElementById("main");
  main.className = "main";

  if (state.tab === "chat") {
    if (tabChanged) {
      main.replaceChildren();
      delete main.dataset.viewFp;
      delete main.dataset.viewTab;
      ui.lastHistoryRevision = null;
      ui.lastContextRevision = null;
      ui.lastHistoryLineCount = -1;
    }
    updateChat(main, "full");
  } else {
    main.querySelector(".chat-shell")?.remove();
    renderMainPanel(main, tabChanged);
  }

  updateApprovalModal();
  prevState = state;
}

function render() {
  scheduleRender(true);
}

function applyLivePatch(patch) {
  if (!state) state = {};
  const keys = [
    "status",
    "chat_busy",
    "chat_streaming",
    "chat_reasoning",
    "chat_tool_running",
    "chat_tool_running_detail",
    "chat_tool_pending",
    "chat_turn_phase",
    "chat_reasoning_compressing",
    "chat_activity_flow",
  ];
  for (const key of keys) {
    if (Object.prototype.hasOwnProperty.call(patch, key)) {
      state[key] = patch[key];
    }
  }
}

const CHAT_PATCH_KEYS = [
  "status",
  "chat_busy",
  "chat_session_id",
  "chat_lines",
  "chat_tool_outputs",
  "chat_history_revision",
  "chat_context_revision",
  "chat_streaming",
  "chat_reasoning",
  "chat_tool_running",
  "chat_tool_running_detail",
  "chat_tool_pending",
  "chat_turn_phase",
  "chat_reasoning_compressing",
  "chat_activity_flow",
  "chat_context_visible",
  "chat_context",
  "chat_pending_approval",
  "approval_dialog",
];

function applyChatPatch(patch) {
  if (!state) state = {};
  const prevSession = state.chat_session_id;
  for (const key of CHAT_PATCH_KEYS) {
    if (Object.prototype.hasOwnProperty.call(patch, key)) {
      state[key] = patch[key];
    }
  }
  if (
    Object.prototype.hasOwnProperty.call(patch, "chat_session_id") &&
    patch.chat_session_id !== prevSession
  ) {
    resetChatUiState();
    ui.sessionList = null;
  }
}

let liveRenderQueued = false;
let chatRenderQueued = false;
let chatRenderTimer = null;

function scheduleLiveRender() {
  if (liveRenderQueued) return;
  liveRenderQueued = true;
  requestAnimationFrame(() => {
    liveRenderQueued = false;
    applyChatLiveRender();
  });
}

function scheduleChatRender() {
  if (chatRenderQueued) return;
  const run = () => {
    chatRenderQueued = true;
    requestAnimationFrame(() => {
      chatRenderQueued = false;
      chatRenderTimer = null;
      applyChatStructuralRender();
    });
  };
  if (state?.chat_busy) {
    if (chatRenderTimer) return;
    chatRenderTimer = setTimeout(run, 120);
    return;
  }
  if (chatRenderTimer) {
    clearTimeout(chatRenderTimer);
    chatRenderTimer = null;
  }
  run();
}

function connectWs() {
  const proto = location.protocol === "https:" ? "wss:" : "ws:";
  ws = new WebSocket(`${proto}//${location.host}/ws`);
  ws.onopen = () => updateStatus();
  ws.onmessage = (ev) => {
    try {
      const data = JSON.parse(ev.data);
      if (data._type === "live") {
        applyLivePatch(data);
        scheduleLiveRender();
        return;
      }
      if (data._type === "chat") {
        applyChatPatch(data);
        scheduleChatRender();
        return;
      }
      state = data;
      ui.lastHistoryRevision = state.chat_history_revision;
      ui.lastContextRevision = state.chat_context_revision;
      ui.lastHistoryLineCount = (state.chat_lines || []).length;
      initThemeFromConfig(state.ui_theme);
      scheduleRender();
    } catch (e) {
      console.error(e);
    }
  };
  ws.onclose = () => {
    updateStatus();
    setTimeout(connectWs, 2000);
  };
}

fetch("/api/state")
  .then((r) => r.json())
  .then((s) => {
    state = s;
    ui.lastHistoryRevision = state.chat_history_revision;
    ui.lastContextRevision = state.chat_context_revision;
    ui.lastHistoryLineCount = (state.chat_lines || []).length;
    initThemeFromConfig(state.ui_theme);
    scheduleRender(true);
  })
  .catch(console.error);

document.getElementById("theme-toggle")?.addEventListener("click", toggleTheme);

connectWs();
