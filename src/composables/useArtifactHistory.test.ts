import { describe, it, expect } from "vitest";
import { useArtifactHistory } from "./useArtifactHistory";

/** 期待する状態を [エントリ列, 現在位置] で書けるようにする小さなヘルパ */
function snapshot(h: ReturnType<typeof useArtifactHistory>): [string[], number] {
  return [h.entries.value, h.index.value];
}

describe("useArtifactHistory", () => {
  it("初期状態は空で、前にも後ろにも進めない", () => {
    const h = useArtifactHistory();
    expect(snapshot(h)).toEqual([[], -1]);
    expect(h.current.value).toBeNull();
    expect(h.canGoBack.value).toBe(false);
    expect(h.canGoForward.value).toBe(false);
  });

  describe("push", () => {
    it("順に積み、現在位置が末尾を指す", () => {
      const h = useArtifactHistory();
      h.push("a");
      h.push("b");
      expect(snapshot(h)).toEqual([["a", "b"], 1]);
      expect(h.current.value).toBe("b");
      expect(h.canGoBack.value).toBe(true);
      expect(h.canGoForward.value).toBe(false);
    });

    it("同じ ID の連続 push は無視する", () => {
      const h = useArtifactHistory();
      h.push("a");
      h.push("a");
      expect(snapshot(h)).toEqual([["a"], 0]);
    });

    it("戻ってから push すると先の履歴を捨てる", () => {
      const h = useArtifactHistory();
      h.push("a");
      h.push("b");
      h.push("c");
      h.moveTo(0);
      h.push("d");
      expect(snapshot(h)).toEqual([["a", "d"], 1]);
      expect(h.canGoForward.value).toBe(false);
    });

    it("往復した結果として同じ ID が複数回並びうる", () => {
      const h = useArtifactHistory();
      h.push("a");
      h.push("b");
      h.push("a");
      expect(snapshot(h)).toEqual([["a", "b", "a"], 2]);
    });
  });

  describe("replace", () => {
    it("空の状態では最初のエントリになる", () => {
      const h = useArtifactHistory();
      h.replace("a");
      expect(snapshot(h)).toEqual([["a"], 0]);
    });

    it("現在位置を差し替えてもスタックは伸びない", () => {
      const h = useArtifactHistory();
      h.push("a");
      h.push("b");
      h.replace("c");
      expect(snapshot(h)).toEqual([["a", "c"], 1]);
      expect(h.canGoBack.value).toBe(true);
    });

    it("先の履歴は捨てる", () => {
      const h = useArtifactHistory();
      h.push("a");
      h.push("b");
      h.push("c");
      h.moveTo(1);
      h.replace("x");
      expect(snapshot(h)).toEqual([["a", "x"], 1]);
    });
  });

  describe("prune", () => {
    it("履歴に無い ID では何も起きない", () => {
      const h = useArtifactHistory();
      h.push("a");
      h.push("b");
      h.prune("z");
      expect(snapshot(h)).toEqual([["a", "b"], 1]);
    });

    it("現在位置より前のエントリを抜くと位置が繰り上がる", () => {
      const h = useArtifactHistory();
      h.push("a");
      h.push("b");
      h.push("c");
      h.prune("a");
      expect(snapshot(h)).toEqual([["b", "c"], 1]);
      expect(h.current.value).toBe("c");
    });

    it("現在位置より後ろのエントリを抜いても位置は動かない", () => {
      const h = useArtifactHistory();
      h.push("a");
      h.push("b");
      h.push("c");
      h.moveTo(0);
      h.prune("c");
      expect(snapshot(h)).toEqual([["a", "b"], 0]);
    });

    it("現在位置そのものを抜くと1つ前へ下がる", () => {
      const h = useArtifactHistory();
      h.push("a");
      h.push("b");
      h.prune("b");
      expect(snapshot(h)).toEqual([["a"], 0]);
      expect(h.current.value).toBe("a");
    });

    it("重複エントリを全て抜いても位置が破綻しない（現在位置が末尾の重複）", () => {
      const h = useArtifactHistory();
      h.push("a");
      h.push("b");
      h.push("a");
      h.prune("a");
      expect(snapshot(h)).toEqual([["b"], 0]);
    });

    it("重複エントリを全て抜いても位置が破綻しない（現在位置が中間）", () => {
      const h = useArtifactHistory();
      h.push("a");
      h.push("b");
      h.push("a");
      h.moveTo(1);
      h.prune("a");
      expect(snapshot(h)).toEqual([["b"], 0]);
      expect(h.current.value).toBe("b");
    });

    it("唯一のエントリを抜くと空に戻る", () => {
      const h = useArtifactHistory();
      h.push("a");
      h.prune("a");
      expect(snapshot(h)).toEqual([[], -1]);
      expect(h.current.value).toBeNull();
      expect(h.canGoBack.value).toBe(false);
      expect(h.canGoForward.value).toBe(false);
    });

    it("空に戻ったあとも push で復帰できる", () => {
      const h = useArtifactHistory();
      h.push("a");
      h.prune("a");
      h.push("b");
      expect(snapshot(h)).toEqual([["b"], 0]);
    });
  });

  describe("moveTo", () => {
    it("範囲外は無視する", () => {
      const h = useArtifactHistory();
      h.push("a");
      h.push("b");
      h.moveTo(-1);
      h.moveTo(2);
      expect(h.index.value).toBe(1);
    });
  });
});
