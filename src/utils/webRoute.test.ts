import { describe, expect, it } from "vitest";
import { buildWebPath, parseWebPath } from "./webRoute";

describe("parseWebPath", () => {
  it("parses the worktrees list page", () => {
    expect(parseWebPath("/worktrees")).toEqual({ page: "worktrees" });
  });

  it("parses a worktree artifact list", () => {
    expect(parseWebPath("/worktrees/issue-326/artifacts")).toEqual({
      page: "list",
      scope: "worktree",
      scopeKey: "issue-326",
    });
  });

  it("parses a worktree artifact view", () => {
    expect(parseWebPath("/worktrees/issue-326/artifacts/flow")).toEqual({
      page: "view",
      scope: "worktree",
      scopeKey: "issue-326",
      artifactId: "flow",
    });
  });

  it("parses a repository artifact list", () => {
    expect(parseWebPath("/repositories/abcd1234/artifacts")).toEqual({
      page: "list",
      scope: "repository",
      scopeKey: "abcd1234",
    });
  });

  it("parses a repository artifact view", () => {
    expect(parseWebPath("/repositories/abcd1234/artifacts/bench")).toEqual({
      page: "view",
      scope: "repository",
      scopeKey: "abcd1234",
      artifactId: "bench",
    });
  });

  it("decodes percent-encoded segments", () => {
    expect(parseWebPath("/worktrees/issue-326/artifacts/%E6%97%A5%E6%9C%AC")).toEqual({
      page: "view",
      scope: "worktree",
      scopeKey: "issue-326",
      artifactId: "日本",
    });
  });

  it("rejects a bare /repositories (no SPA route on the Rust side)", () => {
    expect(parseWebPath("/repositories")).toEqual({ page: "notFound" });
  });

  it("rejects unknown roots", () => {
    expect(parseWebPath("/foo")).toEqual({ page: "notFound" });
    expect(parseWebPath("/")).toEqual({ page: "notFound" });
  });

  it("rejects malformed paths", () => {
    expect(parseWebPath("/worktrees/issue-326/notartifacts")).toEqual({ page: "notFound" });
    expect(parseWebPath("/worktrees/issue-326/artifacts/foo/bar")).toEqual({ page: "notFound" });
  });

  it("rejects an unparseable percent-encoding", () => {
    expect(parseWebPath("/worktrees/issue-326/artifacts/%")).toEqual({ page: "notFound" });
  });
});

describe("buildWebPath", () => {
  it("round-trips through parseWebPath", () => {
    const routes: Parameters<typeof buildWebPath>[0][] = [
      { page: "worktrees" },
      { page: "list", scope: "worktree", scopeKey: "issue-326" },
      { page: "view", scope: "worktree", scopeKey: "issue-326", artifactId: "flow" },
      { page: "list", scope: "repository", scopeKey: "abcd1234" },
      { page: "view", scope: "repository", scopeKey: "abcd1234", artifactId: "bench" },
    ];
    for (const route of routes) {
      expect(parseWebPath(buildWebPath(route))).toEqual(route);
    }
  });

  it("percent-encodes ids that contain slashes", () => {
    const path = buildWebPath({ page: "view", scope: "worktree", scopeKey: "a/b", artifactId: "x" });
    expect(path).toBe("/worktrees/a%2Fb/artifacts/x");
    expect(parseWebPath(path)).toEqual({
      page: "view",
      scope: "worktree",
      scopeKey: "a/b",
      artifactId: "x",
    });
  });
});
