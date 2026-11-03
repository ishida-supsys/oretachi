import { describe, expect, it } from "vitest";
import type { ArtifactComment } from "../types/artifactComment";
import {
  boxFromPoints,
  clamp01,
  commentsForWindow,
  excerptMatches,
  fractionPin,
  pinPointInRect,
  EXCERPT_MAX_CHARS,
  fromFraction,
  hasAnchorPosition,
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

  it("hasAnchorPosition は AI が付けた位置なしのコメントを見分ける", () => {
    expect(hasAnchorPosition(undefined)).toBe(false);
    expect(hasAnchorPosition({})).toBe(false);
    expect(hasAnchorPosition({ excerpt: "見出し" })).toBe(false);
    expect(hasAnchorPosition({ x: 0.5 })).toBe(false);
    expect(hasAnchorPosition({ x: 0.5, y: 0.2 })).toBe(true);
    expect(hasAnchorPosition({ selector: ":scope > p" })).toBe(true);
  });

  it("commentsForWindow: サブウィンドウは自分宛だけ、main は他ウィンドウ宛を位置なしで受ける", () => {
    const mk = (id: string, anchor: ArtifactComment["anchor"]): ArtifactComment => ({
      ...comment(id, 1),
      anchor,
    });
    const all = [
      mk("m", { x: 0.1, y: 0.2, window: "main" }),
      mk("none", {}),
      mk("w1", { x: 0.3, y: 0.4, selector: ":scope > p", excerpt: "本文", window: "w1" }),
      mk("w9", { x: 0.5, y: 0.6, selector: ":scope > div", excerpt: "古い", window: "w9" }),
    ];
    expect(commentsForWindow(all, "w1").map((c) => c.id)).toEqual(["w1"]);
    expect(commentsForWindow(all, "w2")).toEqual([]);
    const main = commentsForWindow(all, "main");
    expect(main.map((c) => c.id)).toEqual(["m", "none", "w1", "w9"]);
    // 自分宛と window 無しはそのまま
    expect(main[0].anchor).toEqual({ x: 0.1, y: 0.2, window: "main" });
    expect(main[1].anchor).toEqual({});
    // 他ウィンドウ宛は位置を落とす（スタックへ回る）
    expect(main[3].anchor).toEqual({ excerpt: "古い", window: "w9" });
    expect(hasAnchorPosition(main[3].anchor)).toBe(false);
    // 元の配列は変えない
    expect(all[3].anchor.selector).toBe(":scope > div");
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

describe("excerptMatches / pinPointInRect / fractionPin", () => {
  it("excerptMatches: 先頭で突き合わせ、空の抜粋は一致扱い", () => {
    expect(excerptMatches(undefined, "何でも")).toBe(true);
    expect(excerptMatches("", "何でも")).toBe(true);
    expect(excerptMatches("ログイン画面", "ログイン画面\n  続き")).toBe(true);
    expect(excerptMatches("ログイン画面", "タスク一覧")).toBe(false);
    // 後ろへ伸びた分は許す（先頭 40 文字だけ比べる）
    const head = "あ".repeat(40);
    expect(excerptMatches(head + "旧", head + "新")).toBe(true);
    // 短い抜粋は全体を比べる。要素のテキストが縮んだら不一致
    expect(excerptMatches("abcdef", "abc")).toBe(false);
  });

  it("pinPointInRect: rx/ry があればそこ、旧形式は point=中央・element=右上", () => {
    const r = { left: 100, top: 50, width: 200, height: 100 };
    expect(pinPointInRect({ kind: "element", rx: 0.25, ry: 0.5 }, r)).toEqual({ x: 150, y: 100 });
    expect(pinPointInRect({ kind: "point" }, r)).toEqual({ x: 200, y: 100 });
    // 旧形式の element は右上（1px 内側）
    expect(pinPointInRect({ kind: "element" }, r)).toEqual({ x: 299, y: 51 });
    // 範囲外の割合は矩形の内側へ収める
    expect(pinPointInRect({ rx: 5, ry: -3 }, r)).toEqual({ x: 299, y: 51 });
  });

  it("fractionPin: 範囲は右上、位置の無いものは null", () => {
    const size = { width: 1000, height: 500 };
    expect(fractionPin({ kind: "point", x: 0.2, y: 0.4, w: 0.5 }, size)).toEqual({ x: 200, y: 200 });
    expect(fractionPin({ kind: "region", x: 0.2, y: 0.4, w: 0.3 }, size)).toEqual({ x: 500, y: 200 });
    expect(fractionPin({ excerpt: "x" }, size)).toBeNull();
  });
});
