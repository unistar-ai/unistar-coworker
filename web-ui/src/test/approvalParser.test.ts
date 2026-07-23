import { describe, it, expect } from "vitest";
import {
  parseApprovalDescription,
  approvalReasonLines,
  classifyReasonDisplayItems,
} from "../tabs/approvals/parser";

describe("parseApprovalDescription / approvalReasonLines", () => {
  it("maps legacy reason=command to friendly text", () => {
    const d =
      "Chat: bash_run — LLM safety review REJECT (reason=gh api repos/foo -q '.name')";
    const parsed = parseApprovalDescription(d);
    expect(parsed.source).toBe("llm-review");
    expect(approvalReasonLines(parsed)).toEqual([
      "Automated safety check flagged this command — see COMMAND above.",
    ]);
  });

  it("shows fallback text from new Rust formatter", () => {
    const d =
      "Chat: bash_run — LLM safety review REJECT (Automated safety check rejected this action — see the command/payload above.)";
    const parsed = parseApprovalDescription(d);
    expect(approvalReasonLines(parsed)[0]).toContain("see the command");
  });

  it("shows policy Reason for harness issue_add_label (not Chat: title)", () => {
    const d =
      "Chat: add label `approval-ui-test` to issue #1 (unistar-ai/unistar-coworker)";
    const parsed = parseApprovalDescription(d, "issue_add_label");
    expect(parsed.source).toBe("human");
    const lines = approvalReasonLines(parsed);
    expect(lines).toHaveLength(1);
    expect(lines[0]).not.toMatch(/^Chat:/);
    expect(lines[0]).toMatch(/label/i);
    expect(lines[0]).toMatch(/confirmation/i);
  });

  it("shows policy Reason for MCP description", () => {
    const d = "Chat: MCP list_issues on mcp[github]";
    const parsed = parseApprovalDescription(d, "mcp_tool");
    expect(parsed.source).toBe("human");
    expect(approvalReasonLines(parsed)[0]).toMatch(/MCP/);
  });

  it("splits multiple semicolon-separated issues", () => {
    const d =
      "Chat: bash_run — LLM safety review REJECT (HIGH_RISK: rm -rf; Use a safer path)";
    const parsed = parseApprovalDescription(d);
    expect(approvalReasonLines(parsed).length).toBe(2);
  });

  it("expands numbered suggestions and classifies risk tags", () => {
    const d =
      "Chat: bash_run — LLM safety review REJECT (HIGH_RISK_COMMAND: 强制推送到 main; #1 使用 `--force-with-lease`; #2 先 fetch 再 rebase; #3 推送到 feature 分支)";
    const parsed = parseApprovalDescription(d);
    const lines = approvalReasonLines(parsed);
    expect(lines.length).toBeGreaterThanOrEqual(3);
    const items = classifyReasonDisplayItems(lines);
    expect(items[0]).toMatchObject({
      kind: "issue",
      riskType: "HIGH_RISK_COMMAND",
    });
    expect(items.filter((i) => i.kind === "suggestion").length).toBeGreaterThanOrEqual(2);
  });
});
