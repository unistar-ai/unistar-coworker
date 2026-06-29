import { describe, it, expect } from "vitest";
import {
  parseMessage,
  parseToolStep,
  buildChatBlocks,
  summarizeToolGroup,
  splitToolStepGroups,
  toolSourceLabel,
  toolMeta,
  parseToolArgsString,
  formatToolArgValue,
  normalizeReasoningText,
} from "../tabs/chat/parser";

describe("parseMessage", () => {
  it("classifies you/assistant/system/error prefixes", () => {
    expect(parseMessage("you> hello", 0).role).toBe("you");
    expect(parseMessage("assistant> reply", 1).role).toBe("assistant");
    expect(parseMessage("system> note", 2).role).toBe("system");
    expect(parseMessage("error> boom", 3).role).toBe("error");
  });

  it("strips prefix from body", () => {
    expect(parseMessage("you> hello world", 0).body).toBe("hello world");
    expect(parseMessage("assistant> **bold**", 1).body).toBe("**bold**");
  });

  it("marks you/assistant as md, others as plain", () => {
    expect(parseMessage("you> x", 0).md).toBe(true);
    expect(parseMessage("assistant> x", 1).md).toBe(true);
    expect(parseMessage("system> x", 2).md).toBe(false);
    expect(parseMessage("error> x", 3).md).toBe(false);
  });

  it("tool step prefixes classify as tool role", () => {
    expect(parseMessage("  ✓ done", 0).role).toBe("tool");
    expect(parseMessage("  → start", 1).role).toBe("tool");
    expect(parseMessage("  ✗ fail", 2).role).toBe("tool");
  });
});

describe("parseToolStep", () => {
  it("parses start step with name + args", () => {
    const s = parseToolStep("  → pr_get_diff(repo=acme/widget, pr_number=42)", 0, {});
    expect(s.kind).toBe("start");
    expect(s.name).toBe("pr_get_diff");
    expect(s.args).toBe("repo=acme/widget, pr_number=42");
  });

  it("parses done step with ms timing", () => {
    const s = parseToolStep("  ✓ pr_get_diff(repo=acme/widget)(120ms)", 0, {});
    expect(s.kind).toBe("done");
    expect(s.ok).toBe(true);
    expect(s.ms).toBe("120");
    expect(s.name).toBe("pr_get_diff");
  });

  it("parses failed step", () => {
    const s = parseToolStep("  ✗ bash_run(ls)(5ms)", 0, {});
    expect(s.kind).toBe("done");
    expect(s.ok).toBe(false);
  });

  it("parses approval-pending", () => {
    const s = parseToolStep("  ⏳ ci_rerun_workflow", 0, {});
    expect(s.kind).toBe("approval-pending");
  });

  it("parses reasoning with stored output", () => {
    const outputs = { 0: "thinking about the problem" };
    const s = parseToolStep("  … reasoning", 0, outputs);
    expect(s.kind).toBe("reasoning");
    expect(s.output).toBe("thinking about the problem");
  });

  it("parses approval resolution", () => {
    const s = parseToolStep("  ✓ approval approved", 0, {});
    expect(s.kind).toBe("approval");
    expect(s.ok).toBe(true);
  });
});

describe("summarizeToolGroup", () => {
  it("summarizes a completed tool group", () => {
    const steps = [
      { kind: "start" as const, text: "pr_get_diff", index: 0, name: "pr_get_diff", args: null },
      { kind: "done" as const, text: "pr_get_diff(120ms)", index: 1, name: "pr_get_diff", args: null, ms: "120", ok: true, output: null },
    ];
    const g = summarizeToolGroup(steps);
    expect(g.toolName).toBe("pr_get_diff");
    expect(g.status).toBe("ok");
    expect(g.ms).toBe("120");
  });

  it("summarizes a failed group", () => {
    const steps = [
      { kind: "start" as const, text: "bash_run", index: 0, name: "bash_run", args: null },
      { kind: "done" as const, text: "bash_run(5ms)", index: 1, name: "bash_run", args: null, ms: "5", ok: false, output: null },
    ];
    expect(summarizeToolGroup(steps).status).toBe("err");
  });

  it("summarizes a pending (approval) group", () => {
    const steps = [
      { kind: "start" as const, text: "ci_rerun_workflow", index: 0, name: "ci_rerun_workflow", args: null },
      { kind: "approval-pending" as const, text: "ci_rerun_workflow", index: 1, ok: null },
    ];
    expect(summarizeToolGroup(steps).status).toBe("pending");
  });

  it("summarizes a running group (started, not done)", () => {
    const steps = [
      { kind: "start" as const, text: "bash_run", index: 0, name: "bash_run", args: null },
    ];
    expect(summarizeToolGroup(steps).status).toBe("running");
  });
});

describe("splitToolStepGroups", () => {
  it("pairs start and done for the same tool", () => {
    const steps = [
      { kind: "start" as const, text: "tool_a", index: 0, name: "tool_a", args: null },
      { kind: "done" as const, text: "tool_a(10ms)", index: 1, name: "tool_a", args: null, ms: "10", ok: true, output: null },
    ];
    const groups = splitToolStepGroups(steps);
    expect(groups.length).toBe(1);
    expect(groups[0].length).toBe(2);
  });

  it("pairs interleaved parallel tools (start a, start b, done a, done b)", () => {
    const steps = [
      { kind: "start" as const, text: "tool_a", index: 0, name: "tool_a", args: null },
      { kind: "start" as const, text: "tool_b", index: 1, name: "tool_b", args: null },
      { kind: "done" as const, text: "tool_a(10ms)", index: 2, name: "tool_a", args: null, ms: "10", ok: true, output: null },
      { kind: "done" as const, text: "tool_b(20ms)", index: 3, name: "tool_b", args: null, ms: "20", ok: true, output: null },
    ];
    const groups = splitToolStepGroups(steps);
    expect(groups.length).toBe(2);
    expect(groups[0].find((s) => s.kind === "start")?.name).toBe("tool_a");
    expect(groups[1].find((s) => s.kind === "start")?.name).toBe("tool_b");
  });
});

describe("toolSourceLabel", () => {
  it("identifies github tools by prefix", () => {
    expect(toolSourceLabel("pr_get_diff")?.source).toBe("github");
    expect(toolSourceLabel("ci_get_logs")?.source).toBe("github");
    expect(toolSourceLabel("issue_add_label")?.source).toBe("github");
  });

  it("identifies local workspace tools", () => {
    expect(toolSourceLabel("bash_run")?.source).toBe("local");
    expect(toolSourceLabel("read_file")?.source).toBe("local");
    expect(toolSourceLabel("grep")?.source).toBe("local");
  });

  it("identifies MCP tools by server prefix", () => {
    const servers = [{ id: "slack", prefix: "slack_" }];
    expect(toolSourceLabel("slack_post_message", servers)?.source).toBe("mcp:slack");
    expect(toolSourceLabel("slack_post_message", servers)?.detail).toBe("post_message");
  });

  it("returns null for unknown tools", () => {
    expect(toolSourceLabel("some_unknown_tool")).toBeNull();
  });
});

describe("toolMeta", () => {
  it("returns icon + label for known tools", () => {
    const m = toolMeta("bash_run");
    expect(m.icon).toBe("⌘");
    expect(m.label).toBe("Bash");
  });

  it("returns default icon for unknown tools", () => {
    const m = toolMeta("custom_tool");
    expect(m.icon).toBe("⚙");
    expect(m.label).toBe("custom_tool");
  });

  it("includes source when resolvable", () => {
    const m = toolMeta("pr_get_diff");
    expect(m.source?.source).toBe("github");
  });
});

describe("parseToolArgsString", () => {
  it("parses key=value pairs", () => {
    const pairs = parseToolArgsString("repo=acme/widget, pr_number=42");
    expect(pairs).toEqual([
      { key: "repo", value: "acme/widget" },
      { key: "pr_number", value: "42" },
    ]);
  });

  it("handles valueless keys", () => {
    const pairs = parseToolArgsString("flag");
    expect(pairs).toEqual([{ key: "flag", value: "" }]);
  });

  it("returns empty for null/empty", () => {
    expect(parseToolArgsString(null)).toEqual([]);
    expect(parseToolArgsString("")).toEqual([]);
    expect(parseToolArgsString("  ")).toEqual([]);
  });
});

describe("formatToolArgValue", () => {
  it("formats pr_number with #", () => {
    expect(formatToolArgValue("pr_number", "42")).toBe("#42");
  });

  it("formats max_bytes in k", () => {
    expect(formatToolArgValue("max_bytes", "4096")).toBe("4k");
  });

  it("passes through short values", () => {
    expect(formatToolArgValue("repo", "acme/widget")).toBe("acme/widget");
  });

  it("truncates long values", () => {
    const long = "x".repeat(40);
    const out = formatToolArgValue("path", long);
    expect(out.length).toBeLessThan(long.length);
    expect(out).toContain("…");
  });
});

describe("normalizeReasoningText", () => {
  it("strips [agent reasoning summary] prefix", () => {
    expect(normalizeReasoningText("[agent reasoning summary] hello")).toBe("hello");
  });

  it("strips reasoning: prefix", () => {
    expect(normalizeReasoningText("reasoning: thinking")).toBe("thinking");
  });

  it("trims whitespace", () => {
    expect(normalizeReasoningText("  hello  ")).toBe("hello");
  });

  it("returns empty for null/undefined", () => {
    expect(normalizeReasoningText(null)).toBe("");
    expect(normalizeReasoningText(undefined)).toBe("");
  });
});

describe("buildChatBlocks", () => {
  it("groups messages and tool groups", () => {
    const lines = [
      "you> summarize PRs",
      "assistant> Let me check.",
      "  → pr_list_open()",
      "  ✓ pr_list_open(50ms)",
      "assistant> Found 3 PRs.",
    ];
    const blocks = buildChatBlocks(lines, {});
    expect(blocks.length).toBe(4);
    expect(blocks[0].type).toBe("message");
    expect(blocks[1].type).toBe("message");
    expect(blocks[2].type).toBe("tool-group");
    expect(blocks[3].type).toBe("message");
  });

  it("emits a standalone reasoning block when output is present and alone", () => {
    const lines = ["  … thinking", "assistant> reply"];
    const outputs = { 0: "deep reasoning text" };
    const blocks = buildChatBlocks(lines, outputs);
    expect(blocks[0].type).toBe("reasoning");
    expect(blocks[0].reasoningText).toBe("deep reasoning text");
    expect(blocks[1].type).toBe("message");
  });

  it("folds reasoning after a tool into the tool group, not a standalone block", () => {
    const lines = [
      "  → pr_get_overview(repo=acme/widget, pr_number=1)",
      "  ✓ pr_get_overview(repo=acme/widget, pr_number=1)(50ms)",
      "  … follow-up thought",
      "assistant> done",
    ];
    const outputs = { 2: "follow-up reasoning body" };
    const blocks = buildChatBlocks(lines, outputs);
    expect(blocks.some((b) => b.type === "reasoning")).toBe(false);
    const tg = blocks.find((b) => b.type === "tool-group");
    expect(tg).toBeDefined();
    expect(tg?.steps?.some((s) => s.kind === "reasoning")).toBe(true);
  });

  it("merges 3+ consecutive tool groups into one batch strip", () => {
    const lines = [
      "  → tool_a()",
      "  ✓ tool_a()(10ms)",
      "  → tool_b()",
      "  ✓ tool_b()(10ms)",
      "  → tool_c()",
      "  ✓ tool_c()(10ms)",
      "assistant> done",
    ];
    const blocks = buildChatBlocks(lines, {});
    expect(blocks.length).toBe(2);
    expect(blocks[0].type).toBe("tool-batch");
    expect(blocks[0].groups?.length).toBe(3);
  });

  it("folds interim assistant into tool batch when surrounded by tool steps", () => {
    const lines = [
      "  → pr_list_open()",
      "assistant> Checking open PRs now.",
      "  ✓ pr_list_open(50ms)",
    ];
    const blocks = buildChatBlocks(lines, {});
    // The interim assistant line should be inside the tool batch, not a
    // standalone message — so we get exactly one tool-batch block.
    expect(blocks.length).toBe(1);
    expect(blocks[0].type).toBe("tool-group");
    const interim = blocks[0].steps?.find((s) => s.kind === "interim");
    expect(interim).toBeDefined();
    expect(interim?.text).toBe("Checking open PRs now.");
  });

  it("does not treat final assistant reply as interim", () => {
    const lines = [
      "  → pr_list_open()",
      "  ✓ pr_list_open(50ms)",
      "assistant> Here is the summary.",
    ];
    const blocks = buildChatBlocks(lines, {});
    // Final assistant after tool done is a standalone message, not interim.
    expect(blocks.length).toBe(2);
    expect(blocks[0].type).toBe("tool-group");
    expect(blocks[1].type).toBe("message");
    expect(blocks[1].message?.role).toBe("assistant");
  });
});
