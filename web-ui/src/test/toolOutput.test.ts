import { describe, it, expect } from "vitest";
import {
  looksLikeDiff,
  formatDiffHtml,
  formatToolOutputHtml,
  looksLikeJson,
  tryPrettyJson,
  looksLikeBashOutput,
  collapseLongOutput,
  ToolStepOutput,
} from "../tabs/chat/toolOutput";
import { renderToStaticMarkup } from "react-dom/server";
import { createElement } from "react";

describe("collapseLongOutput", () => {
  it("keeps head and tail with an omission marker", () => {
    const lines = Array.from({ length: 20 }, (_, i) => `line ${i}`).join("\n");
    const out = collapseLongOutput(lines, 2, 2);
    expect(out).toContain("line 0");
    expect(out).toContain("line 19");
    expect(out).toContain("lines omitted");
    expect(out).not.toContain("line 10");
  });
});

describe("looksLikeBashOutput", () => {
  it("detects exit and stdout prefixes", () => {
    expect(looksLikeBashOutput("stdout: ok\nexit: 0")).toBe(true);
    expect(looksLikeBashOutput("plain text")).toBe(false);
  });
});

describe("looksLikeDiff", () => {
  it("detects a `diff --git` header", () => {
    expect(looksLikeDiff("diff --git a/foo b/foo\n@@ -1 +1 @@\n-x\n+x")).toBe(true);
  });

  it("detects an `@@` hunk marker without a diff --git header", () => {
    expect(looksLikeDiff("@@ -1,3 +1,3 @@\n context\n-removed\n+added")).toBe(true);
  });

  it("returns false for plain stdout / error logs", () => {
    expect(looksLikeDiff("stdout: hello\nexit: 0")).toBe(false);
    expect(looksLikeDiff("some random error message")).toBe(false);
    expect(looksLikeDiff("")).toBe(false);
  });
});

describe("formatDiffHtml", () => {
  it("tints added lines green and removed lines red with a gutter marker", () => {
    const html = formatDiffHtml("+added line\n-removed line");
    expect(html).toContain("diff-add");
    expect(html).toContain("diff-del");
    expect(html).toContain(">+</span>added line");
    expect(html).toContain(">-</span>removed line");
  });

  it("styles hunk headers and file headers distinctly", () => {
    const html = formatDiffHtml(
      "diff --git a/foo b/foo\n--- a/foo\n+++ b/foo\n@@ -1 +1 @@",
    );
    expect(html).toContain("diff-meta");
    expect(html).toContain("diff-hunk");
    expect(html).toContain("diff-header");
  });

  it("escapes HTML in diff content", () => {
    const html = formatDiffHtml("+<script>x</script>");
    // The raw tag must be escaped, not live.
    expect(html).not.toContain("<script>");
    expect(html).toContain("&lt;script&gt;");
  });
});

describe("formatToolOutputHtml (unchanged behaviour)", () => {
  it("highlights exit codes", () => {
    const html = formatToolOutputHtml("exit: 0");
    expect(html).toContain("out-exit ok");
  });
  it("escapes HTML in normal output", () => {
    const html = formatToolOutputHtml("<b>not bold</b>");
    expect(html).toContain("&lt;b&gt;");
  });
});

describe("looksLikeJson", () => {
  it("detects JSON objects and arrays", () => {
    expect(looksLikeJson('{"a":1}')).toBe(true);
    expect(looksLikeJson("[1,2,3]")).toBe(true);
    expect(looksLikeJson('  {"a":1}  ')).toBe(true);
  });
  it("rejects non-JSON and invalid JSON", () => {
    expect(looksLikeJson("stdout: hello")).toBe(false);
    expect(looksLikeJson("{not valid json")).toBe(false);
    expect(looksLikeJson("")).toBe(false);
    expect(looksLikeJson("plain text")).toBe(false);
  });
});

describe("tryPrettyJson", () => {
  it("pretty-prints compact JSON with 2-space indent", () => {
    expect(tryPrettyJson('{"a":1,"b":2}')).toBe('{\n  "a": 1,\n  "b": 2\n}');
  });
  it("returns the original text on invalid JSON", () => {
    expect(tryPrettyJson("{not json")).toBe("{not json");
  });
});

describe("ToolStepOutput", () => {
  it("strips tool_result envelope and renders skill markdown body", () => {
    const content = `tool_result(skill_load):
args: {
  "name": "gh-cli"
}

### gh-cli
# GitHub CLI (gh)`;
    const html = renderToStaticMarkup(
      createElement(ToolStepOutput, {
        output: content,
        outputKey: "t1",
        toolName: "skill_load",
        inline: true,
        preferMarkdown: true,
      }),
    );
    expect(html).not.toContain("tool_result(skill_load)");
    expect(html).not.toContain('"name": "gh-cli"');
    expect(html).toContain("GitHub CLI (gh)");
    expect(html).toContain("tool-output-markdown");
  });

  it("renders markdown tool body as plain pre when toolMarkdown is off", () => {
    const content = `tool_result(skill_load):
args: {"name":"gh-cli"}

### gh-cli
# GitHub CLI (gh)`;
    const html = renderToStaticMarkup(
      createElement(ToolStepOutput, {
        output: content,
        outputKey: "t1b",
        toolName: "skill_load",
        inline: true,
        preferMarkdown: false,
      }),
    );
    expect(html).not.toContain("tool-output-markdown");
    expect(html).toContain("tool-output");
    expect(html).toContain("GitHub CLI (gh)");
  });

  it("renders bash stdout without review/cwd noise", () => {
    const content = `tool_result(bash_run):
args: {"command":"ls"}

review: APPROVE (AUTO)
cwd: /tmp
exit: 0 (5ms)

stdout:
README.md`;
    const html = renderToStaticMarkup(
      createElement(ToolStepOutput, {
        output: content,
        outputKey: "t2",
        toolName: "bash_run",
        inline: true,
      }),
    );
    expect(html).not.toContain("tool_result");
    expect(html).not.toContain("review:");
    expect(html).toContain("README.md");
    expect(html).toContain("exit 0");
  });

  it("renders read_file with line numbers", () => {
    const content = `tool_result(read_file):
args: {"path":"a.ts"}

1|const x = 1
2|const y = 2`;
    const html = renderToStaticMarkup(
      createElement(ToolStepOutput, {
        output: content,
        outputKey: "t3",
        toolName: "read_file",
        inline: true,
      }),
    );
    expect(html).toContain("tool-read-file-ln");
    expect(html).toContain("const x = 1");
  });
});
