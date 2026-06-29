import { describe, it, expect } from "vitest";
import { render } from "@testing-library/react";
import CodeBlock from "../components/CodeBlock";

describe("CodeBlock", () => {
  it("renders a language badge when lang is provided", () => {
    const { container } = render(<CodeBlock code="echo hello" lang="bash" />);
    const badge = container.querySelector(".md-code-lang");
    expect(badge).not.toBeNull();
    expect(badge?.textContent).toBe("bash");
  });

  it("omits the badge when lang is empty", () => {
    const { container } = render(<CodeBlock code="echo hello" />);
    expect(container.querySelector(".md-code-lang")).toBeNull();
  });

  it("wraps code in a pre element", () => {
    const { container } = render(<CodeBlock code="let x = 1" lang="rust" />);
    const pre = container.querySelector("pre");
    expect(pre).not.toBeNull();
    expect(pre?.textContent).toContain("let x = 1");
  });

  it("applies syntax highlight spans for bash comments", () => {
    const { container } = render(<CodeBlock code="# a comment" lang="bash" />);
    const tokComment = container.querySelector(".tok-comment");
    expect(tokComment).not.toBeNull();
    expect(tokComment?.textContent).toContain("# a comment");
  });

  it("applies syntax highlight spans for rust keywords", () => {
    const { container } = render(<CodeBlock code="fn main() {}" lang="rust" />);
    const tokKw = container.querySelector(".tok-kw");
    expect(tokKw).not.toBeNull();
    expect(tokKw?.textContent).toBe("fn");
  });

  it("applies syntax highlight spans for json keys", () => {
    const { container } = render(
      <CodeBlock code={`{"key": "value"}`} lang="json" />,
    );
    const tokKey = container.querySelector(".tok-key");
    expect(tokKey).not.toBeNull();
  });

  it("escapes HTML in code content (no raw injection)", () => {
    const { container } = render(
      <CodeBlock code={`<script>alert(1)</script>`} lang="text" />,
    );
    // The <script> tag must not appear as a live DOM element.
    expect(container.querySelectorAll("script").length).toBe(0);
    // But the text content should still contain the literal characters.
    expect(container.textContent).toContain("<script>");
  });
});
