import { describe, expect, it } from "vitest";
import {
  ARTIFACT_COMMENT_PROBE_CHANGED_MARKER,
  ARTIFACT_COMMENT_PROBE_JS,
  ARTIFACT_COMMENT_PROBE_MARKER,
  ARTIFACT_COMMENT_PROBE_RESULT_MARKER,
  sanitizeFrameAnchor,
  sanitizeFramePositions,
} from "./artifactFrameComment";

describe("ARTIFACT_COMMENT_PROBE_JS", () => {
  it("構文として正しい（srcdoc の inline script として実行できる）", () => {
    expect(() => new Function("document", "window", "parent", ARTIFACT_COMMENT_PROBE_JS)).not.toThrow();
  });

  it("サーバーに載る文字列に </script> を含まない（srcdoc を壊さない）", () => {
    expect(ARTIFACT_COMMENT_PROBE_JS.toLowerCase()).not.toContain("</script");
  });

  /** DOM を持たない node でも、メッセージ口とマーカーが期待どおり組み上がることだけ確かめる */
  it("親からの locate 問い合わせに、マーカー付きで答える", () => {
    const listeners: Record<string, ((e: unknown) => void)[]> = {};
    const sent: unknown[] = [];
    const parent = { postMessage: (m: unknown) => sent.push(m) };
    const root = {
      scrollWidth: 1000,
      scrollHeight: 2000,
      contains: () => true,
      querySelector: () => null,
      querySelectorAll: () => [],
    };
    const doc = {
      body: root,
      documentElement: { scrollWidth: 1000, scrollHeight: 2000 },
      addEventListener: () => {},
      elementsFromPoint: () => [],
    };
    const win = {
      innerWidth: 800,
      innerHeight: 600,
      scrollX: 0,
      scrollY: 0,
      addEventListener: (type: string, fn: (e: unknown) => void) => {
        (listeners[type] ??= []).push(fn);
      },
    };
    new Function(
      "document",
      "window",
      "parent",
      "MutationObserver",
      "requestAnimationFrame",
      ARTIFACT_COMMENT_PROBE_JS,
    )(doc, win, parent, class { observe() {} }, () => 0);

    // 他の送信元や無関係なメッセージは無視する
    listeners.message[0]({ source: {}, data: { [ARTIFACT_COMMENT_PROBE_MARKER]: true, id: 1, op: "locate", items: [] } });
    listeners.message[0]({ source: parent, data: { hello: true } });
    expect(sent).toHaveLength(0);

    // 座標フォールバック（selector なし）の x,y は文書基準の割合で、ビューポートへ直して返る
    listeners.message[0]({
      source: parent,
      data: {
        [ARTIFACT_COMMENT_PROBE_MARKER]: true,
        id: 7,
        op: "locate",
        items: [
          { id: "a", anchor: { x: 0.5, y: 0.1 } },
          { id: "b", anchor: { x: 0.5, y: 0.9 } },
          { id: "c", anchor: {} },
        ],
      },
    });
    expect(sent).toHaveLength(1);
    const res = sent[0] as Record<string, any>;
    expect(res[ARTIFACT_COMMENT_PROBE_RESULT_MARKER]).toBe(true);
    expect(res.id).toBe(7);
    expect(res.positions.a).toEqual({ x: 500, y: 200, orphan: false });
    // 画面の外（スクロールで隠れている）と位置を持たないものは null
    expect(res.positions.b).toBeNull();
    expect(res.positions.c).toBeNull();
    expect(ARTIFACT_COMMENT_PROBE_CHANGED_MARKER).toBeTruthy();
  });
});

describe("sanitizeFrameAnchor", () => {
  it("既知のキーだけを残し、割合は 0..1 に丸める", () => {
    const out = sanitizeFrameAnchor({
      kind: "point",
      x: 2,
      y: -1,
      rx: 0.5,
      selector: ":scope > div",
      excerpt: "hello",
      evil: "<script>",
    });
    expect(out).toEqual({ kind: "point", x: 1, y: 0, rx: 0.5, selector: ":scope > div", excerpt: "hello" });
  });

  it("型の違う値・未知の kind・巨大な文字列を落とす", () => {
    const out = sanitizeFrameAnchor({ kind: "weird", x: "1", y: Number.NaN, selector: "a".repeat(5000) });
    expect(out?.kind).toBeUndefined();
    expect(out?.x).toBeUndefined();
    expect(out?.selector).toHaveLength(1000);
  });

  it("オブジェクトでない・空なら null", () => {
    expect(sanitizeFrameAnchor(null)).toBeNull();
    expect(sanitizeFrameAnchor("x")).toBeNull();
    expect(sanitizeFrameAnchor({})).toBeNull();
  });
});

describe("sanitizeFramePositions", () => {
  it("要求した ID だけを返し、壊れた値は null にする", () => {
    const out = sanitizeFramePositions(
      { a: { x: 1, y: 2, orphan: true }, b: { x: "1", y: 2 }, zzz: { x: 0, y: 0 } },
      ["a", "b", "c"],
    );
    expect(out).toEqual({ a: { x: 1, y: 2, orphan: true }, b: null, c: null });
  });

  it("prototype のキーを拾わない", () => {
    expect(sanitizeFramePositions({}, ["toString", "hasOwnProperty"])).toEqual({ toString: null, hasOwnProperty: null });
  });
});
