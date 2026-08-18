import { describe, it, expect } from "vitest";
import { resolveExternalLink } from "./externalLink";

describe("resolveExternalLink", () => {
  it("accepts http and https", () => {
    expect(resolveExternalLink("https://example.com")).toBe("https://example.com");
    expect(resolveExternalLink("http://example.com/a?b=1#c")).toBe("http://example.com/a?b=1#c");
    expect(resolveExternalLink("HTTPS://EXAMPLE.COM")).toBe("HTTPS://EXAMPLE.COM");
  });

  it("trims surrounding whitespace", () => {
    expect(resolveExternalLink("  https://example.com  ")).toBe("https://example.com");
  });

  it("rejects in-page anchors and relative paths", () => {
    expect(resolveExternalLink("#section")).toBeNull();
    expect(resolveExternalLink("./docs/a.md")).toBeNull();
    expect(resolveExternalLink("../a.md")).toBeNull();
    expect(resolveExternalLink("/abs/path")).toBeNull();
  });

  it("rejects local paths and non-http schemes", () => {
    expect(resolveExternalLink("C:\Windows\notepad.exe")).toBeNull();
    expect(resolveExternalLink("file:///C:/Windows")).toBeNull();
    expect(resolveExternalLink("mailto:a@b.com")).toBeNull();
    expect(resolveExternalLink("javascript:alert(1)")).toBeNull();
    // スキームだけ http に見せかけた変種も落とす
    expect(resolveExternalLink("javascript:void('https://e.com')")).toBeNull();
  });

  it("rejects empty, whitespace-only and non-string input", () => {
    expect(resolveExternalLink("")).toBeNull();
    expect(resolveExternalLink("   ")).toBeNull();
    expect(resolveExternalLink("https://")).toBeNull();
    expect(resolveExternalLink(null)).toBeNull();
    expect(resolveExternalLink(undefined)).toBeNull();
  });
});
