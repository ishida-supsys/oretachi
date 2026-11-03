import { describe, expect, it } from "vitest";
import type { ArtifactComment } from "../types/artifactComment";
import {
  boxFromPoints,
  clamp01,
  EXCERPT_MAX_CHARS,
  fromFraction,
  isDrag,
  makeExcerpt,
  placePopup,
  toFraction,
  unresolvedCount,
  visibleComments,
} from "./commentAnchor";

function comment(id: string, createdAt: number, resolved = false): ArtifactComment {
  return { id, author: "user", body: id, anchor: {}, createdAt, resolved, replies: [] };
}

describe("commentAnchor", () => {
  it("clamp01 は範囲外と非数を丸める", () => {
    expect(clamp01(-1)).toBe(0);
    expect(clamp01(2)).toBe(1);
    expect(clamp01(Number.NaN)).toBe(0);
    expect(clamp01(0.3)).toBe(0.3);
  });

  it("boxFromPoints はドラッグの向きに依らない", () => {
    expect(boxFromPoints({ x: 10, y: 20 }, { x: 4, y: 50 })).toEqual({ left: 4, top: 20, width: 6, height: 30 });
  });

  it("isDrag はしきい値未満をクリックとみなす", () => {
    expect(isDrag({ x: 0, y: 0 }, { x: 2, y: 2 })).toBe(false);
    expect(isDrag({ x: 0, y: 0 }, { x: 10, y: 0 })).toBe(true);
  });

  it("toFraction / fromFraction は往復し、潰れたルートでも NaN を出さない", () => {
    const root = { left: 100, top: 50, width: 400, height: 200 };
    const f = toFraction({ x: 300, y: 150 }, root);
    expect(f).toEqual({ x: 0.5, y: 0.5 });
    expect(fromFraction(f, root)).toEqual({ x: 200, y: 100 });
    expect(toFraction({ x: 1, y: 1 }, { left: 0, top: 0, width: 0, height: 0 })).toEqual({ x: 0, y: 0 });
  });

  it("makeExcerpt は空白を畳んで切り詰める", () => {
    expect(makeExcerpt("  a \n\n b\t c ")).toBe("a b c");
    expect(makeExcerpt(null)).toBe("");
    const long = makeExcerpt("x".repeat(EXCERPT_MAX_CHARS + 50));
    expect(long.length).toBe(EXCERPT_MAX_CHARS + 1);
    expect(long.endsWith("…")).toBe(true);
  });

  it("visibleComments は未解決だけを付けた順に並べる", () => {
    const list = [comment("b", 2), comment("a", 1), comment("c", 3, true)];
    expect(visibleComments(list).map((c) => c.id)).toEqual(["a", "b"]);
    expect(unresolvedCount(list)).toBe(2);
    expect(visibleComments(undefined)).toEqual([]);
  });

  it("placePopup はルートからはみ出さない", () => {
    const popup = { width: 300, height: 200 };
    const root = { width: 800, height: 600 };
    expect(placePopup({ x: 100, y: 100 }, popup, root)).toEqual({ x: 108, y: 100 });
    // 右端に近いときは左側へ出す
    expect(placePopup({ x: 700, y: 100 }, popup, root).x).toBe(392);
    // 下端に近いときは上へ寄せる
    expect(placePopup({ x: 100, y: 550 }, popup, root).y).toBe(400);
    // ルートより大きくても負にならない
    expect(placePopup({ x: 10, y: 10 }, popup, { width: 100, height: 100 })).toEqual({ x: 0, y: 0 });
  });
});
