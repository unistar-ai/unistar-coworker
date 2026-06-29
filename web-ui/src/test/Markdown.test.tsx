import { describe, it, expect } from "vitest";
import { render } from "@testing-library/react";
import Markdown from "../components/Markdown";

describe("Markdown", () => {
  it("renders a paragraph", () => {
    const { container } = render(<Markdown>{"hello world"}</Markdown>);
    expect(container.querySelector("p")?.textContent).toBe("hello world");
  });

  it("renders a code block with language badge", () => {
    const md = "```bash\necho hello\n```";
    const { container } = render(<Markdown>{md}</Markdown>);
    expect(container.querySelector(".md-code-lang")?.textContent).toBe("bash");
    expect(container.querySelector("pre")?.textContent).toContain("echo hello");
  });

  it("renders inline code", () => {
    const { container } = render(<Markdown>{"use `x` here"}</Markdown>);
    const code = container.querySelector("code");
    expect(code).not.toBeNull();
    expect(code?.textContent).toBe("x");
  });

  it("renders a GFM table", () => {
    const md = "| a | b |\n| --- | --- |\n| 1 | 2 |";
    const { container } = render(<Markdown>{md}</Markdown>);
    expect(container.querySelector("table")).not.toBeNull();
    expect(container.querySelectorAll("th").length).toBe(2);
  });

  it("renders a task list checkbox", () => {
    const md = "- [x] done\n- [ ] todo";
    const { container } = render(<Markdown>{md}</Markdown>);
    const checkboxes = container.querySelectorAll('input[type="checkbox"]');
    expect(checkboxes.length).toBe(2);
    expect((checkboxes[0] as HTMLInputElement).checked).toBe(true);
    expect((checkboxes[1] as HTMLInputElement).checked).toBe(false);
  });

  it("renders links with target=_blank and rel=noopener", () => {
    const md = "[example](https://example.com)";
    const { container } = render(<Markdown>{md}</Markdown>);
    const a = container.querySelector("a");
    expect(a?.getAttribute("href")).toBe("https://example.com");
    expect(a?.getAttribute("target")).toBe("_blank");
    expect(a?.getAttribute("rel")).toContain("noopener");
  });

  it("neutralizes javascript: URLs (XSS protection)", () => {
    const md = "[click](javascript:alert(1))";
    const { container } = render(<Markdown>{md}</Markdown>);
    const a = container.querySelector("a");
    // The href must NOT be a javascript: URL.
    expect(a?.getAttribute("href")).not.toContain("javascript:");
    expect(a?.getAttribute("href")).toBe("#");
  });

  it("neutralizes data: URLs", () => {
    const md = "[x](data:text/html,<script>alert(1)</script>)";
    const { container } = render(<Markdown>{md}</Markdown>);
    const a = container.querySelector("a");
    expect(a?.getAttribute("href")).toBe("#");
    // No live script element should be injected.
    expect(container.querySelectorAll("script").length).toBe(0);
  });

  it("allows mailto: URLs", () => {
    const md = "[email](mailto:user@example.com)";
    const { container } = render(<Markdown>{md}</Markdown>);
    const a = container.querySelector("a");
    expect(a?.getAttribute("href")).toBe("mailto:user@example.com");
  });

  it("renders blockquote", () => {
    const md = "> quoted text";
    const { container } = render(<Markdown>{md}</Markdown>);
    expect(container.querySelector("blockquote")).not.toBeNull();
  });

  it("applies the is-streaming class when streaming=true", () => {
    const { container } = render(
      <Markdown streaming={true}>{"hi"}</Markdown>,
    );
    expect(
      container.querySelector(".prose-chat")?.classList.contains("is-streaming"),
    ).toBe(true);
  });

  it("renders bullet lists with ul/li", () => {
    const md = [
      "across 4 e2e test specs:",
      "- `jwt-signer.spec.ts`",
      "- `solace-consume.spec.ts`",
      "- `solace-log.spec.ts`",
      "- `zipkin.spec.ts`",
    ].join("\n");
    const { container } = render(<Markdown>{md}</Markdown>);
    const ul = container.querySelector("ul");
    expect(ul).not.toBeNull();
    expect(container.querySelectorAll("ul > li").length).toBe(4);
    const style = ul ? getComputedStyle(ul) : null;
    expect(style?.listStyleType).toBe("disc");
  });

  it("renders nested bullet lists (ul in ul)", () => {
    const md = ["- parent", "  - child a", "  - child b", "- other"].join("\n");
    const { container } = render(<Markdown>{md}</Markdown>);
    expect(container.querySelectorAll("ul").length).toBe(2);
    expect(container.querySelectorAll("ul ul > li").length).toBe(2);
    expect(container.querySelector("ul > li > ul")).not.toBeNull();
  });

  it("renders ordered list with nested bullets", () => {
    const md = ["1. first", "   - sub a", "   - sub b", "2. second"].join("\n");
    const { container } = render(<Markdown>{md}</Markdown>);
    expect(container.querySelector("ol")).not.toBeNull();
    expect(container.querySelector("ol > li > ul")).not.toBeNull();
    expect(container.querySelectorAll("ol > li > ul > li").length).toBe(2);
  });

  it("renders nested ordered lists", () => {
    const md = ["1. one", "   1. inner", "   2. inner two", "2. two"].join("\n");
    const { container } = render(<Markdown>{md}</Markdown>);
    expect(container.querySelectorAll("ol").length).toBe(2);
    expect(container.querySelector("ol > li > ol > li")).not.toBeNull();
  });
});
