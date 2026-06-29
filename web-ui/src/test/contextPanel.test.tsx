import { describe, it, expect } from "vitest";
import { render } from "@testing-library/react";
import { toolsBodyAsMarkdown, SkillFrontmatter } from "../tabs/chat/ContextPanel";
import type { SkillBlock } from "../store/protocol";

describe("toolsBodyAsMarkdown", () => {
  it("returns a placeholder for empty body", () => {
    expect(toolsBodyAsMarkdown("")).toBe("_(no tool schema text)_");
    expect(toolsBodyAsMarkdown("   \n  ")).toBe("_(no tool schema text)_");
  });

  it("pretty-prints valid JSON inside a ```json fenced block", () => {
    const md = toolsBodyAsMarkdown('{"name":"bash_run","args":{"cmd":"ls"}}');
    expect(md.startsWith("```json\n")).toBe(true);
    expect(md.endsWith("\n```")).toBe(true);
    // Pretty-printed with 2-space indent.
    expect(md).toContain('"name": "bash_run"');
    expect(md).toContain('  "args": {');
  });

  it("returns non-JSON body verbatim (markdown prose fallback)", () => {
    const prose = "## Tools\n- bash_run: run a command\n- read_file: read a file";
    expect(toolsBodyAsMarkdown(prose)).toBe(prose);
  });

  it("handles JSON arrays", () => {
    const md = toolsBodyAsMarkdown('[{"a":1},{"b":2}]');
    expect(md.startsWith("```json\n")).toBe(true);
    expect(md).toContain('"a": 1');
  });
});

describe("SkillFrontmatter", () => {
  it("renders nothing when the skill has no frontmatter metadata", () => {
    const sk: SkillBlock = { name: "noop", tokens: 10, body: "body" };
    const { container } = render(<SkillFrontmatter sk={sk} />);
    expect(container.querySelector(".skill-frontmatter")).toBeNull();
  });

  it("renders description, always-on badge, skills/tools as chips", () => {
    const sk: SkillBlock = {
      name: "github-ops-tone",
      tokens: 120,
      body: "...",
      description: "Secretary tone for all chat replies",
      always: true,
      skills: ["ci-triage", "branch-health"],
      tools: ["pr_get_diff"],
    };
    const { container } = render(<SkillFrontmatter sk={sk} />);
    const fm = container.querySelector(".skill-frontmatter");
    expect(fm).not.toBeNull();
    expect(fm?.textContent).toContain("Secretary tone for all chat replies");
    expect(container.querySelector(".skill-frontmatter-badge.is-always")).not.toBeNull();
    // Tools are individual chips.
    const toolChips = container.querySelectorAll(".skill-frontmatter-chip.is-tool");
    expect(toolChips.length).toBe(1);
    expect(toolChips[0].textContent).toBe("pr_get_diff");
    // Skills are individual chips.
    const skillChips = container.querySelectorAll(".skill-frontmatter-chip.is-skill");
    expect(skillChips.length).toBe(2);
    expect(skillChips[0].textContent).toBe("ci-triage");
    expect(skillChips[1].textContent).toBe("branch-health");
  });

  it("renders argument-hint and intent triggers when present", () => {
    const sk: SkillBlock = {
      name: "my-prs",
      tokens: 140,
      body: "...",
      description: "Author-focused open PR status",
      argument_hint: "Author filter or repo",
      tools: ["pr_list_open", "pr_get_status_batch"],
      intent_phrases: ["my pr", "my open"],
      intent_bonus_keywords: ["@me"],
    };
    const { container } = render(<SkillFrontmatter sk={sk} />);
    const fm = container.querySelector(".skill-frontmatter");
    expect(fm).not.toBeNull();
    expect(fm?.textContent).toContain("Author filter or repo");
    expect(container.querySelector(".skill-frontmatter-arghint-value")?.textContent).toBe(
      "Author filter or repo",
    );
    // Triggers rendered as chips (phrases + bonus keywords).
    const triggers = container.querySelectorAll(".skill-frontmatter-chip.is-trigger");
    expect(triggers.length).toBe(2);
    const bonus = container.querySelectorAll(".skill-frontmatter-chip.is-bonus");
    expect(bonus.length).toBe(1);
    expect(bonus[0].textContent).toBe("+@me");
  });

  it("omits the always-on badge when always is false", () => {
    const sk: SkillBlock = {
      name: "ci-triage",
      tokens: 80,
      body: "...",
      description: "Classify CI failures",
      always: false,
      tools: ["pr_get_ci_snapshot"],
    };
    const { container } = render(<SkillFrontmatter sk={sk} />);
    expect(container.querySelector(".skill-frontmatter-badge.is-always")).toBeNull();
    // Description + tools still render.
    expect(container.querySelector(".skill-frontmatter")?.textContent).toContain(
      "Classify CI failures",
    );
    expect(container.querySelector(".skill-frontmatter")?.textContent).toContain(
      "pr_get_ci_snapshot",
    );
  });
});
