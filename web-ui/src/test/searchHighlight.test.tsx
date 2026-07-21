import { describe, it, expect } from "vitest";
import { highlightSearchText, highlightReactNodes } from "../tabs/chat/searchHighlight";
import { render } from "@testing-library/react";

describe("highlightSearchText", () => {
  it("wraps matches in mark.chat-search-hit", () => {
    const nodes = highlightSearchText("hello world hello", "hello");
    const { container } = render(<div>{nodes}</div>);
    const marks = container.querySelectorAll("mark.chat-search-hit");
    expect(marks.length).toBe(2);
    expect(marks[0].textContent).toBe("hello");
  });

  it("returns original text when query empty", () => {
    expect(highlightSearchText("abc", "")).toBe("abc");
  });
});

describe("highlightReactNodes", () => {
  it("highlights nested text nodes", () => {
    const nodes = highlightReactNodes(
      <>
        <span>alpha beta</span>
        <em>beta gamma</em>
      </>,
      "beta",
    );
    const { container } = render(<div>{nodes}</div>);
    const marks = container.querySelectorAll("mark.chat-search-hit");
    expect(marks.length).toBe(2);
  });
});
