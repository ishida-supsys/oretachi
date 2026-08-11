import { describe, it, expect } from "vitest";
import { extractUrlArtifacts } from "./artifactUrl";

describe("extractUrlArtifacts", () => {
  it("picks only text/uri-list entries", () => {
    const list = [
      { id: "a", type: "text/markdown", title: "doc", content: "# hi" },
      { id: "b", type: "text/uri-list", title: "o/r#1", content: "https://github.com/o/r/issues/1" },
    ];
    expect(extractUrlArtifacts(list)).toEqual([
      { id: "b", title: "o/r#1", url: "https://github.com/o/r/issues/1" },
    ]);
  });

  it("accepts content_type as well as type", () => {
    const list = [{ id: "b", content_type: "text/uri-list", title: "x", content: "https://e.com" }];
    expect(extractUrlArtifacts(list)).toEqual([
      { id: "b", title: "x", url: "https://e.com" },
    ]);
  });

  it("uses the first line and trims whitespace", () => {
    const list = [
      { id: "b", type: "text/uri-list", title: "", content: "  https://e.com/a  \nignored\n" },
    ];
    expect(extractUrlArtifacts(list)).toEqual([
      { id: "b", title: "https://e.com/a", url: "https://e.com/a" },
    ]);
  });

  it("skips malformed entries", () => {
    const list = [
      null,
      "nope",
      { id: "", type: "text/uri-list", content: "https://e.com" },
      { id: "c", type: "text/uri-list", content: "   " },
    ];
    expect(extractUrlArtifacts(list)).toEqual([]);
  });

  it("rejects non-http(s) schemes", () => {
    const list = [
      { id: "a", type: "text/uri-list", title: "js", content: "javascript:alert(1)" },
      { id: "b", type: "text/uri-list", title: "file", content: "file:///C:/Windows" },
      { id: "c", type: "text/uri-list", title: "mail", content: "mailto:x@example.com" },
      { id: "d", type: "text/uri-list", title: "ok", content: "https://e.com" },
    ];
    expect(extractUrlArtifacts(list)).toEqual([
      { id: "d", title: "ok", url: "https://e.com" },
    ]);
  });
});
