import { describe, it, expect } from "vitest";
import { sortArtifacts, filterArtifacts } from "./artifactList";
import type { ArtifactMeta } from "../types/artifact";

function meta(id: string, title: string, updatedAt: number, contentType = "text/markdown"): ArtifactMeta {
  return { id, title, content_type: contentType, created_at: 0, updated_at: updatedAt };
}

describe("sortArtifacts", () => {
  const list = [
    meta("a", "A", 100),
    meta("b", "B", 300),
    meta("c", "C", 200),
  ];

  it("ピン止めが無ければ updated_at 降順", () => {
    expect(sortArtifacts(list, () => false).map((a) => a.id)).toEqual(["b", "c", "a"]);
  });

  it("ピン止めは updated_at より優先して先頭に来る", () => {
    expect(sortArtifacts(list, (id) => id === "a").map((a) => a.id)).toEqual(["a", "b", "c"]);
  });

  it("ピン止め同士は updated_at 降順で並ぶ", () => {
    expect(sortArtifacts(list, (id) => id === "a" || id === "c").map((a) => a.id))
      .toEqual(["c", "a", "b"]);
  });

  it("元の配列を破壊しない", () => {
    const original = [...list];
    sortArtifacts(list, () => false);
    expect(list).toEqual(original);
  });
});

describe("filterArtifacts", () => {
  const list = [
    meta("a", "設計メモ", 100, "text/markdown"),
    meta("b", "Sales table", 200, "text/csv"),
    meta("c", "graph", 300, "application/vnd.ant.mermaid"),
  ];

  it("空クエリなら元の配列をそのまま返す", () => {
    expect(filterArtifacts(list, "   ")).toBe(list);
  });

  it("タイトルの部分一致（大文字小文字を無視）", () => {
    expect(filterArtifacts(list, "SALES").map((a) => a.id)).toEqual(["b"]);
  });

  it("content_type の部分一致も拾う", () => {
    expect(filterArtifacts(list, "csv").map((a) => a.id)).toEqual(["b"]);
    expect(filterArtifacts(list, "mermaid").map((a) => a.id)).toEqual(["c"]);
  });

  it("日本語タイトルにも一致する", () => {
    expect(filterArtifacts(list, "設計").map((a) => a.id)).toEqual(["a"]);
  });

  it("一致しなければ空配列", () => {
    expect(filterArtifacts(list, "zzz")).toEqual([]);
  });
});
