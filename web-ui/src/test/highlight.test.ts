import { describe, it, expect } from "vitest";
import { resolveLang, SHIKI_LANGS } from "../lib/lang";

describe("highlight resolveLang", () => {
  it("resolves canonical shiki languages", () => {
    expect(resolveLang("bash")).toBe("bash");
    expect(resolveLang("json")).toBe("json");
    expect(resolveLang("rust")).toBe("rust");
    expect(resolveLang("javascript")).toBe("javascript");
    expect(resolveLang("typescript")).toBe("typescript");
    expect(resolveLang("shell")).toBe("shell");
  });

  it("maps common aliases to a shiki language", () => {
    expect(resolveLang("sh")).toBe("bash");
    expect(resolveLang("zsh")).toBe("bash");
    expect(resolveLang("rs")).toBe("rust");
    expect(resolveLang("js")).toBe("javascript");
    expect(resolveLang("ts")).toBe("typescript");
    expect(resolveLang("py")).toBe("python");
    expect(resolveLang("yml")).toBe("yaml");
    expect(resolveLang("golang")).toBe("go");
  });

  it("is case-insensitive", () => {
    expect(resolveLang("Bash")).toBe("bash");
    expect(resolveLang("JSON")).toBe("json");
    expect(resolveLang("JS")).toBe("javascript");
    expect(resolveLang("Python")).toBe("python");
  });

  it("returns null for unsupported languages (regex fallback path)", () => {
    expect(resolveLang("text")).toBeNull();
    expect(resolveLang("ruby")).toBeNull();
    expect(resolveLang("html")).toBeNull();
    expect(resolveLang("")).toBeNull();
    expect(resolveLang(undefined)).toBeNull();
  });

  it("SHIKI_LANGS lists exactly the supported set", () => {
    expect(SHIKI_LANGS).toEqual([
      "bash",
      "shell",
      "json",
      "rust",
      "javascript",
      "typescript",
      "python",
      "go",
      "yaml",
      "sql",
      "toml",
      "diff",
    ]);
  });
});
