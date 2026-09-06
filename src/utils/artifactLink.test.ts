import { describe, it, expect } from "vitest";
import { parseArtifactLink, formatArtifactLink } from "./artifactLink";

describe("parseArtifactLink", () => {
  it("短縮形はスコープ・ID を持たない", () => {
    expect(parseArtifactLink("artifact:report-2026")).toEqual({
      scope: null,
      id: null,
      artifactId: "report-2026",
    });
  });

  it("短縮形はパーセントエンコードを解く", () => {
    expect(parseArtifactLink("artifact:%E5%9B%B3")?.artifactId).toBe("図");
  });

  it("worktree スコープの完全形を解析する", () => {
    expect(parseArtifactLink("artifact://worktree/1788694152541-q2yv/diagram")).toEqual({
      scope: "worktree",
      id: "1788694152541-q2yv",
      artifactId: "diagram",
    });
  });

  it("repository スコープの ID は絶対パスへ復元する", () => {
    expect(parseArtifactLink("artifact://repository/X%3A%5Cdevel%5Coretachi/report")).toEqual({
      scope: "repository",
      id: "X:\\devel\\oretachi",
      artifactId: "report",
    });
  });

  it("スキームの大小は問わない", () => {
    expect(parseArtifactLink("ARTIFACT:report")?.artifactId).toBe("report");
  });

  it("前後の空白を無視する", () => {
    expect(parseArtifactLink("  artifact:report  ")?.artifactId).toBe("report");
  });

  it.each([
    ["artifact 以外のスキーム", "https://example.com/a"],
    ["スキームなし", "report-2026"],
    ["null", null],
    ["undefined", undefined],
    ["空の短縮形", "artifact:"],
    ["未知のスコープ", "artifact://project/abc/report"],
    ["セグメント不足", "artifact://worktree/abc"],
    ["セグメント過多", "artifact://worktree/abc/def/ghi"],
    ["空のスコープ ID", "artifact://worktree//report"],
    ["空のアーティファクト ID", "artifact://worktree/abc/"],
    ["壊れたエスケープ", "artifact:%"],
  ])("%s は null を返す", (_label, href) => {
    expect(parseArtifactLink(href)).toBeNull();
  });

  it.each([
    ["artifact:../../etc/passwd"],
    ["artifact:%2E%2E%2Ffoo"],
    ["artifact:a/b"],
    ["artifact://worktree/abc/%2E%2E"],
    ["artifact://worktree/abc/%2Fetc%2Fpasswd"],
  ])("パス脱出を許さない: %s", (href) => {
    expect(parseArtifactLink(href)).toBeNull();
  });
});

describe("formatArtifactLink", () => {
  it("短縮形へ戻す", () => {
    expect(formatArtifactLink({ scope: null, id: null, artifactId: "report" })).toBe(
      "artifact:report",
    );
  });

  it("repository スコープの ID をエンコードして戻す", () => {
    expect(
      formatArtifactLink({ scope: "repository", id: "X:\\devel\\oretachi", artifactId: "report" }),
    ).toBe("artifact://repository/X%3A%5Cdevel%5Coretachi/report");
  });
});
