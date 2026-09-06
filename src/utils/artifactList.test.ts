import { describe, it, expect } from "vitest";
import {
  sortArtifacts,
  filterArtifacts,
  pushHistory,
  pruneHistory,
  canGoBack,
  canGoForward,
  createHistory,
  type SelectionHistory,
} from "./artifactList";
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

describe("pushHistory", () => {
  it("空の履歴に積むと index 0 になる", () => {
    expect(pushHistory(createHistory(), "a")).toEqual({ entries: ["a"], index: 0 });
  });

  it("現在位置と同じ ID なら履歴を変えない", () => {
    const h: SelectionHistory = { entries: ["a", "b"], index: 1 };
    expect(pushHistory(h, "b")).toBe(h);
  });

  it("戻った状態から積むと進む側が捨てられる", () => {
    const h: SelectionHistory = { entries: ["a", "b", "c"], index: 0 };
    expect(pushHistory(h, "d")).toEqual({ entries: ["a", "d"], index: 1 });
  });

  it("同じ ID でも現在位置でなければ積む（a→b→a を辿れる）", () => {
    const h: SelectionHistory = { entries: ["a", "b"], index: 1 };
    expect(pushHistory(h, "a")).toEqual({ entries: ["a", "b", "a"], index: 2 });
  });
});

describe("pruneHistory", () => {
  it("削除済み ID を取り除き、現在位置を詰める", () => {
    const h: SelectionHistory = { entries: ["a", "b", "c"], index: 2 };
    expect(pruneHistory(h, ["a", "c"])).toEqual({ entries: ["a", "c"], index: 1 });
  });

  it("現在位置より前が消えると index が前へ寄る", () => {
    const h: SelectionHistory = { entries: ["a", "b", "c"], index: 2 };
    expect(pruneHistory(h, ["b", "c"])).toEqual({ entries: ["b", "c"], index: 1 });
  });

  it("現在位置そのものが消えると、それ以前で生き残った最後の要素を指す", () => {
    const h: SelectionHistory = { entries: ["a", "b", "c"], index: 1 };
    expect(pruneHistory(h, ["a", "c"])).toEqual({ entries: ["a", "c"], index: 0 });
  });

  it("現在位置以前が全滅すると index は -1 になり、進む側だけが残る", () => {
    const h: SelectionHistory = { entries: ["a", "b", "c"], index: 1 };
    const pruned = pruneHistory(h, ["c"]);
    expect(pruned).toEqual({ entries: ["c"], index: -1 });
    // 進む先が残っているので「進む」だけ有効
    expect(canGoBack(pruned)).toBe(false);
    expect(canGoForward(pruned)).toBe(true);
  });

  it("間の ID が消えてできた隣接重複を畳み込む（戻るが無反応にならない）", () => {
    // "a" 表示中に "b" が消える。畳まないと ["a","a"] になり、戻っても表示が変わらない
    const h: SelectionHistory = { entries: ["a", "b", "a"], index: 2 };
    expect(pruneHistory(h, ["a"])).toEqual({ entries: ["a"], index: 0 });
  });

  it("畳み込んでも現在位置は畳んだ後の要素を指す", () => {
    const h: SelectionHistory = { entries: ["a", "b", "a", "c"], index: 3 };
    expect(pruneHistory(h, ["a", "c"])).toEqual({ entries: ["a", "c"], index: 1 });
  });

  it("離れた位置の同一 ID は重複として畳まない", () => {
    const h: SelectionHistory = { entries: ["a", "b", "a"], index: 2 };
    expect(pruneHistory(h, ["a", "b"])).toEqual({ entries: ["a", "b", "a"], index: 2 });
  });

  it("全部消えると空の履歴になる", () => {
    const h: SelectionHistory = { entries: ["a", "b"], index: 1 };
    expect(pruneHistory(h, [])).toEqual(createHistory());
  });
});

describe("canGoBack / canGoForward", () => {
  it("空の履歴ではどちらも無効", () => {
    expect(canGoBack(createHistory())).toBe(false);
    expect(canGoForward(createHistory())).toBe(false);
  });

  it("先頭では戻れず、末尾では進めない", () => {
    expect(canGoBack({ entries: ["a", "b"], index: 0 })).toBe(false);
    expect(canGoForward({ entries: ["a", "b"], index: 0 })).toBe(true);
    expect(canGoBack({ entries: ["a", "b"], index: 1 })).toBe(true);
    expect(canGoForward({ entries: ["a", "b"], index: 1 })).toBe(false);
  });
});
