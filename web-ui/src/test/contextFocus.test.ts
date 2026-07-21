import { describe, it, expect } from "vitest";
import {
  buildContextToolFocus,
  findContextToolMessageIndex,
  formatArgsShortFromRecord,
  toolGroupOutputHint,
} from "../tabs/chat/contextFocus";

function toolMsg(name: string, args: Record<string, unknown>, body: string) {
  const argsJson = JSON.stringify(args);
  return {
    role: "tool",
    content: `tool_result(${name}):\nargs: ${argsJson}\n\n${body}`,
  };
}

describe("formatArgsShortFromRecord", () => {
  it("mirrors short key=value form", () => {
    expect(
      formatArgsShortFromRecord({ path: "src/lib.rs", max_bytes: 1000 }),
    ).toBe("path=src/lib.rs, max_bytes=1000");
  });
});

describe("findContextToolMessageIndex", () => {
  const messages = [
    { role: "user", content: "hi" },
    toolMsg("read_file", { path: "a.ts" }, "const a = 1"),
    toolMsg("bash_run", { command: "ls" }, "a.ts\nb.ts"),
    toolMsg("read_file", { path: "b.ts" }, "const b = 2"),
  ];

  it("returns -1 for ambiguous name-only focus", () => {
    expect(
      findContextToolMessageIndex(messages, {
        toolName: "read_file",
        argsShort: null,
        outputHint: null,
      }),
    ).toBe(-1);
  });

  it("matches by args short form", () => {
    expect(
      findContextToolMessageIndex(messages, {
        toolName: "read_file",
        argsShort: "path=b.ts",
        outputHint: null,
      }),
    ).toBe(3);
  });

  it("matches truncated line args with ellipsis against full transcript args", () => {
    const longCmd =
      "gh api repos/unistar-ai/unistar-coworker/commits --paginate -q '.[] | .sha'";
    const msgs = [toolMsg("bash_run", { command: longCmd }, "exit: 0\nstdout:\nok")];
    expect(
      findContextToolMessageIndex(msgs, {
        toolName: "bash_run",
        argsShort: "command=gh api repos/unistar-ai/uni…",
        outputHint: null,
      }),
    ).toBe(0);
  });

  it("matches by output hint when args collide", () => {
    const dup = [
      toolMsg("read_file", { path: "same.ts" }, "version one content"),
      toolMsg("read_file", { path: "same.ts" }, "version two content"),
    ];
    expect(
      findContextToolMessageIndex(dup, {
        toolName: "read_file",
        argsShort: "path=same.ts",
        outputHint: "version two content",
      }),
    ).toBe(1);
  });

  it("matches unique tool by name alone", () => {
    expect(
      findContextToolMessageIndex(messages, {
        toolName: "bash_run",
        argsShort: null,
        outputHint: null,
      }),
    ).toBe(2);
  });
});

describe("toolGroupOutputHint", () => {
  it("takes last done output body without tool_result envelope", () => {
    expect(
      toolGroupOutputHint([
        { kind: "start" },
        {
          kind: "done",
          output: `tool_result(bash_run):
args: {"command":"ls"}

exit: 0
stdout:
README.md`,
        },
      ]),
    ).toContain("README.md");
    expect(
      toolGroupOutputHint([
        {
          kind: "done",
          output: `tool_result(bash_run):
args: {"command":"ls"}

exit: 0
stdout:
README.md`,
        },
      ]),
    ).not.toMatch(/^tool_result/);
  });

  it("takes last done output", () => {
    expect(
      toolGroupOutputHint([
        { kind: "start" },
        { kind: "done", output: "first" },
        { kind: "done", output: "second" },
      ]),
    ).toBe("second");
  });
});

describe("buildContextToolFocus", () => {
  it("rebuilds argsShort from full transcript args", () => {
    const focus = buildContextToolFocus({
      toolName: "bash_run",
      status: "ok",
      ms: "1",
      args: "command=gh api repos/uni…",
      steps: [
        {
          kind: "done",
          index: 0,
          ok: true,
          name: "bash_run",
          text: "",
          output: `tool_result(bash_run):
args: {"command":"gh api repos/unistar-ai/unistar-coworker/commits"}

ok`,
        },
      ],
    });
    expect(focus.toolName).toBe("bash_run");
    expect(focus.argsShort).toContain("command=");
    expect(focus.argsShort).toContain("gh api repos/unistar-ai");
    expect(focus.argsShort).toMatch(/…$/);
    expect(focus.outputHint).toBe("ok");
  });
});
