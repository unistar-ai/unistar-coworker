import { describe, it, expect } from "vitest";
import { splitStreaming } from "../tabs/chat/streamSplit";

describe("splitStreaming", () => {
  it("returns empty split for empty input", () => {
    expect(splitStreaming("")).toEqual({ stable: "", unstable: "" });
  });

  it("keeps a single short line entirely unstable (no stable prefix yet)", () => {
    const { stable, unstable } = splitStreaming("Hello");
    expect(stable).toBe("");
    expect(unstable).toBe("Hello");
  });

  it("keeps two short lines entirely unstable", () => {
    const { stable, unstable } = splitStreaming("Hello\nworld");
    expect(stable).toBe("");
    expect(unstable).toBe("Hello\nworld");
  });

  it("splits at the last paragraph boundary: stable gets the paragraph, tail unstable", () => {
    const text = "First paragraph done.\n\nSecond incomplete";
    const { stable, unstable } = splitStreaming(text);
    expect(stable).toBe("First paragraph done.\n\n");
    expect(unstable).toBe("Second incomplete");
  });

  it("moves an unclosed code fence into the unstable tail", () => {
    // The first paragraph is closed; the second opens a ``` fence that is
    // never closed. After splitting at the paragraph boundary the fence lives
    // in the unstable tail, so it can't swallow the rest as a code block.
    const text = "Intro done.\n\n```bash\ncargo build";
    const { stable, unstable } = splitStreaming(text);
    // Stable must be the completed paragraph (trailing whitespace tolerated).
    expect(stable.trim()).toBe("Intro done.");
    // The unstable tail must include the fence opener and the code line.
    expect(unstable).toContain("```bash");
    expect(unstable).toContain("cargo build");
    // Stable must NOT contain an unclosed fence.
    expect(stable).not.toContain("```");
  });

  it("does not treat a closed code fence as unclosed", () => {
    const text = "```bash\ncargo build\n```\n\nfree text";
    const { stable, unstable } = splitStreaming(text);
    // Closed fence is fine to keep in stable.
    expect(stable).toContain("```bash");
    expect(stable).toContain("cargo build");
    expect(unstable).toBe("free text");
  });

  it("handles many lines without a blank line by keeping the last ~2 lines unstable", () => {
    const text = "line1\nline2\nline3\nline4\nline5";
    const { stable, unstable } = splitStreaming(text);
    expect(stable.length).toBeGreaterThan(0);
    // The tail should contain the last couple of lines.
    expect(unstable).toContain("line5");
    expect(unstable.split("\n").length).toBeLessThanOrEqual(3);
  });

  it("stable part never ends inside an unclosed fence (multi-paragraph)", () => {
    const text = "Para one.\n\n```rust\nfn main() {\n    // still typing";
    const { stable, unstable } = splitStreaming(text);
    expect(stable.trim()).toBe("Para one.");
    expect(unstable).toContain("```rust");
    expect(unstable).toContain("fn main()");
  });

  it("pulls an unclosed fence opener from stable into unstable when the fence starts in stable", () => {
    // No blank line: the fence opener lands in the stable prefix. The splitter
    // must move it into unstable so the stable Markdown render doesn't show an
    // open code block that swallows subsequent text.
    const text = "intro line\n```bash\ncargo build\nstill typing";
    const { stable, unstable } = splitStreaming(text);
    // Stable should not contain the fence opener.
    expect(stable).not.toContain("```");
    // Unstable should contain the fence and the code.
    expect(unstable).toContain("```bash");
    expect(unstable).toContain("cargo build");
  });
});
