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
