import { describe, expect, it } from "vitest";
import {
  parseAskUserBody,
  parseShellTranscript,
  pickToolRowSubtitle,
  preferArgBlock,
  prepareToolStepDisplay,
  resolveToolArgPairs,
  shouldShowInlineArgChips,
  shouldShowInlineCommandBlock,
  toolArgSubtitle,
  toolCollapsedSummary,
  toolOutputPreview,
  toolRowTitle,
} from "../tabs/chat/toolDisplay";

describe("toolArgSubtitle", () => {
  it("formats grep pattern and path", () => {
    expect(toolArgSubtitle("grep", "pattern=foo, path=src/")).toBe("foo · src/");
  });

  it("formats web_fetch url", () => {
    expect(toolArgSubtitle("web_fetch", "url=https://example.com")).toBe(
      "https://example.com",
    );
  });
});

describe("toolOutputPreview", () => {
  it("strips envelope and previews bash stdout", () => {
    const raw = `tool_result(bash_run):
args: {"command":"ls"}

review: APPROVE (AUTO)
cwd: /tmp
exit: 0 (12ms)

stdout:
README.md`;
    expect(toolOutputPreview("bash_run", raw)).toBe("README.md");
  });

  it("previews read_file path header", () => {
    const raw = `tool_result(read_file):
args: {"path":"src/main.rs"}

path: src/main.rs (lines 1-10 of 100) [utf-8, LF]
1|fn main() {}`;
    expect(toolOutputPreview("read_file", raw)).toBe("src/main.rs");
  });

  it("previews ask_user answer", () => {
    const raw = `tool_result(ask_user):
args: {"question":"Which?"}

User answered:
acme/widget`;
    expect(toolOutputPreview("ask_user", raw)).toBe("acme/widget");
  });
});

describe("prepareToolStepDisplay", () => {
  it("uses shell layout for bash output", () => {
    const body = `review: APPROVE (AUTO)
cwd: /tmp
exit: 0 (5ms)

stdout:
ok`;
    const p = prepareToolStepDisplay(
      "bash_run",
      `tool_result(bash_run):\nargs: {}\n\n${body}`,
    );
    expect(p.display).toBe("shell");
    expect(p.body).toContain("stdout:");
  });

  it("uses markdown for skill_load", () => {
    const p = prepareToolStepDisplay(
      "skill_load",
      'tool_result(skill_load):\nargs: {"name":"x"}\n\n### x\nBody',
    );
    expect(p.display).toBe("markdown");
    expect(p.body).toContain("### x");
  });

  it("strips read_file path header from body", () => {
    const p = prepareToolStepDisplay(
      "read_file",
      'tool_result(read_file):\nargs: {"path":"a.ts"}\n\npath: a.ts (lines 1-2 of 2)\n1|x',
    );
    expect(p.display).toBe("read_file");
    expect(p.body).not.toMatch(/^path:/);
    expect(p.body).toContain("1|x");
  });

  it("uses grep display mode", () => {
    const p = prepareToolStepDisplay(
      "grep",
      'tool_result(grep):\nargs: {}\n\nsrc/a.ts:10:match',
    );
    expect(p.display).toBe("grep");
    expect(p.body).toContain("src/a.ts:10:match");
  });
});

describe("toolCollapsedSummary", () => {
  it("prefers row subtitle over line count", () => {
    const summary = toolCollapsedSummary({
      toolName: "read_file",
      status: "ok",
      ms: "5",
      args: "path=src/main.rs",
      steps: [
        {
          kind: "done",
          text: "",
          index: 1,
          output: "tool_result(read_file):\nargs: {}\n\npath: src/main.rs\n1|fn main()",
        },
      ],
    });
    expect(summary).toBe("src/main.rs");
  });
});

describe("parseShellTranscript", () => {
  it("splits stdout and stderr", () => {
    const parts = parseShellTranscript(`review: APPROVE (AUTO)
cwd: /tmp
exit: 1 (8ms)

stdout:
ok

stderr:
fail`);
    expect(parts?.exit).toBe("1 (8ms)");
    expect(parts?.stdout).toBe("ok");
    expect(parts?.stderr).toBe("fail");
  });
});

describe("parseAskUserBody", () => {
  it("extracts question, options, and answer", () => {
    const body = `Awaiting user answer.

Question: Pick one?

Options:
  1. a
  2. b

User answered:
a`;
    const p = parseAskUserBody(body);
    expect(p.question).toBe("Pick one?");
    expect(p.options).toEqual(["a", "b"]);
    expect(p.answer).toBe("a");
  });
});

describe("inline detail visibility", () => {
  it("hides arg chips duplicated in row subtitle", () => {
    expect(
      shouldShowInlineArgChips([{ key: "name", value: "gh-cli" }], "gh-cli"),
    ).toBe(false);
    expect(
      shouldShowInlineArgChips([{ key: "path", value: "src/lib.rs" }], "other"),
    ).toBe(true);
  });

  it("hides command block when row already shows command", () => {
    expect(
      shouldShowInlineCommandBlock(
        "bash_run",
        [{ key: "command", value: "ls -la" }],
        "ls -la",
      ),
    ).toBe(false);
  });
});

describe("pickToolRowSubtitle", () => {
  it("prefers args while running", () => {
    const sub = pickToolRowSubtitle({
      toolName: "grep",
      status: "running",
      ms: null,
      args: "pattern=foo, path=src",
      steps: [],
    });
    expect(sub).toBe("foo · src");
  });
});

describe("toolRowTitle", () => {
  it("localizes common tool names", () => {
    expect(toolRowTitle("read_file", "Read", "ok")).toBe("读取文件");
    expect(toolRowTitle("bash_run", "Bash", "ok")).toBe("执行命令");
  });
});

describe("resolveToolArgPairs", () => {
  it("recovers full args from tool_result transcript instead of truncated line args", () => {
    const longCmd =
      "gh api repos/unistar-ai/unistar-coworker/commits --paginate -q '.[] | {sha:.sha[0:7]}'";
    const pairs = resolveToolArgPairs({
      toolName: "bash_run",
      status: "ok",
      ms: "12",
      args: "command=gh api repos/unistar-ai/uni…",
      steps: [
        {
          kind: "done",
          index: 1,
          ok: true,
          name: "bash_run",
          text: "",
          output: `tool_result(bash_run):
args: ${JSON.stringify({ command: longCmd })}

exit: 0
stdout:
ok`,
        },
      ],
    });
    expect(pairs).toEqual([{ key: "command", value: longCmd }]);
    expect(pairs[0].value).not.toContain("…");
  });

  it("falls back to line args when output has no args block", () => {
    const pairs = resolveToolArgPairs({
      toolName: "glob",
      status: "ok",
      ms: null,
      args: "pattern=**/*.ts",
      steps: [{ kind: "done", index: 0, ok: true, name: "glob", text: "", output: "a.ts\nb.ts" }],
    });
    expect(pairs).toEqual([{ key: "pattern", value: "**/*.ts" }]);
  });

  it("preferArgBlock treats long and multiline values as blocks", () => {
    expect(preferArgBlock("command", "ls")).toBe(true);
    expect(preferArgBlock("repo", "a/b")).toBe(false);
    expect(preferArgBlock("repo", "x".repeat(80))).toBe(true);
  });
});
