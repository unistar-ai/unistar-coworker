import { describe, it, expect } from "vitest";
import {
  parseContextToolTranscript,
  contextToolPreview,
} from "../tabs/chat/contextToolResult";

describe("parseContextToolTranscript", () => {
  it("parses tool_result with args and body", () => {
    const content = `tool_result(skill_load):
args: {"name":"pr-review"}

### pr-review
Skill body here`;
    const p = parseContextToolTranscript(content);
    expect(p.kind).toBe("result");
    expect(p.toolName).toBe("skill_load");
    expect(p.ok).toBe(true);
    expect(p.args).toEqual({ name: "pr-review" });
    expect(p.argsPretty).toContain('"name": "pr-review"');
    expect(p.body).toContain("### pr-review");
    expect(p.body).toContain("Skill body here");
  });

  it("parses pretty-printed multi-line args (store transcript format)", () => {
    const content = `tool_result(skill_load):
args: {
  "name": "gh-cli"
}

### gh-cli
GitHub CLI body`;
    const p = parseContextToolTranscript(content);
    expect(p.args).toEqual({ name: "gh-cli" });
    expect(p.argsPretty).toBe('{\n  "name": "gh-cli"\n}');
    expect(p.body).toContain("### gh-cli");
    expect(p.body).toContain("GitHub CLI body");
  });

  it("parses tool_error", () => {
    const p = parseContextToolTranscript("tool_error(bash_run):\nargs: {}\n\nERROR: denied");
    expect(p.kind).toBe("error");
    expect(p.toolName).toBe("bash_run");
    expect(p.ok).toBe(false);
    expect(p.body).toContain("ERROR");
  });

  it("parses tool_user_question_pending (ask_user)", () => {
    const content = `tool_user_question_pending(ask_user, question_id=00000000-0000-0000-0000-000000000000):
args: {
  "question": "Which repo?",
  "options": ["acme/widget", "acme/api"]
}

Awaiting user answer.

Question: Which repo?

Options:
  1. acme/widget
  2. acme/api`;
    const p = parseContextToolTranscript(content);
    expect(p.kind).toBe("ask_user");
    expect(p.toolName).toBe("ask_user");
    expect(p.ok).toBe(true);
    expect(p.args).toEqual({
      question: "Which repo?",
      options: ["acme/widget", "acme/api"],
    });
    expect(p.body).toContain("Question: Which repo?");
  });

  it("parses answered ask_user tool_result", () => {
    const content = `tool_result(ask_user):
args: {"question":"Which repo?"}

User answered:
acme/widget`;
    const p = parseContextToolTranscript(content);
    expect(p.kind).toBe("result");
    expect(p.toolName).toBe("ask_user");
    expect(p.body).toContain("User answered:");
    expect(p.body).toContain("acme/widget");
  });

  it("parses summarized tool_result", () => {
    const p = parseContextToolTranscript("[summarized tool_result skill_load]\npreview…");
    expect(p.kind).toBe("summarized");
    expect(p.toolName).toBe("skill_load");
    expect(p.body).toContain("preview");
  });

  it("falls back to plain for unstructured text", () => {
    const raw = "some legacy tool output";
    const p = parseContextToolTranscript(raw);
    expect(p.kind).toBe("plain");
    expect(p.toolName).toBeNull();
    expect(p.body).toBe(raw);
  });
});

describe("contextToolPreview", () => {
  it("includes localized title and arg/body snippet", () => {
    const preview = contextToolPreview(
      'tool_result(web_fetch):\nargs: {"url":"https://x"}\n\nOK: 200',
      80,
    );
    expect(preview).toContain("获取网页");
    expect(preview).toMatch(/https:\/\/x|OK/);
  });

  it("prefers command subtitle for shell tools", () => {
    const body =
      'tool_result(bash_run):\nargs: {"command":"ls -la"}\n\n' + "line\n".repeat(10);
    const preview = contextToolPreview(body);
    expect(preview).toContain("执行命令");
    expect(preview).toContain("ls -la");
  });
});
