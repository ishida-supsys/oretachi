import { describe, expect, it } from "vitest";
import { reactive } from "vue";
import {
  ARTIFACT_COMMENT_PROBE_CHANGED_MARKER,
  ARTIFACT_COMMENT_PROBE_JS,
  ARTIFACT_COMMENT_PROBE_MARKER,
  ARTIFACT_COMMENT_PROBE_RESULT_MARKER,
  buildLocateItems,
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
      removeEventListener: () => {},
      elementsFromPoint: () => [],
    };
    const win = {
      innerWidth: 800,
      innerHeight: 600,
      removeEventListener: () => {},
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
    )(doc, win, parent, class { observe() {} disconnect() {} }, () => 0);

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

describe("buildLocateItems", () => {
  /** postMessage は構造化複製を使う。Vue の reactive（Proxy）のままでは DataCloneError になる */
  it("Vue の reactive なコメントを渡しても structuredClone できる", () => {
    const comments = reactive([
      { id: "a", anchor: { kind: "point", x: 0.5, y: 0.5, selector: "#root > div" } },
    ]);
    const items = buildLocateItems(comments as never);
    expect(() => structuredClone(items)).not.toThrow();
    expect(structuredClone(items)).toEqual([
      { id: "a", anchor: { kind: "point", x: 0.5, y: 0.5, selector: "#root > div" } },
    ]);
  });
});

/** 要素のスタブ。locate が触る最小限だけを持つ */
function stubEl(opts: {
  rect: { left: number; top: number; width: number; height: number };
  text?: string;
  parent?: unknown;
}) {
  const r = opts.rect;
  return {
    getBoundingClientRect: () => ({ ...r, right: r.left + r.width, bottom: r.top + r.height }),
    textContent: opts.text ?? "",
    parentElement: opts.parent ?? null,
  };
}

/** プローブを起動して、locate を 1 回問い合わせた結果（{ id: 位置 }）を返す */
function locateWith(
  items: { id: string; anchor: Record<string, unknown> }[],
  elements: Record<string, unknown>,
  styles: Map<unknown, { overflowX: string; overflowY: string }> = new Map(),
) {
  const listeners: Record<string, ((e: unknown) => void)[]> = {};
  const sent: Record<string, any>[] = [];
  const parent = { postMessage: (m: Record<string, any>) => sent.push(m) };
  const body = {
    scrollWidth: 1000,
    scrollHeight: 2000,
    contains: () => true,
    querySelector: (sel: string) => elements[sel] ?? null,
    querySelectorAll: () => [],
  };
  const doc = {
    body,
    documentElement: { scrollWidth: 1000, scrollHeight: 2000 },
    addEventListener: () => {},
    removeEventListener: () => {},
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
    removeEventListener: () => {},
  };
  new Function(
    "document",
    "window",
    "parent",
    "MutationObserver",
    "getComputedStyle",
    ARTIFACT_COMMENT_PROBE_JS,
  )(doc, win, parent, class { observe() {} disconnect() {} }, (el: unknown) => styles.get(el) ?? { overflowX: "visible", overflowY: "visible" });
  listeners.message[0]({
    source: parent,
    data: { [ARTIFACT_COMMENT_PROBE_MARKER]: true, id: 1, op: "locate", items },
  });
  return sent[0].positions as Record<string, { x: number; y: number; orphan: boolean } | null>;
}

describe("ARTIFACT_COMMENT_PROBE_JS: ページ切り替え・パンへの対応", () => {
  const rect = { left: 100, top: 100, width: 200, height: 100 };

  it("ドラッグのコメントは、範囲の右上（要素の rx/ry）にピンを置く", () => {
    const out = locateWith(
      [{ id: "a", anchor: { kind: "element", selector: "#p", rx: 0.25, ry: 0.5, excerpt: "本文" } }],
      { "#p": stubEl({ rect, text: "本文" }) },
    );
    expect(out.a).toEqual({ x: 150, y: 150, orphan: false });
  });

  it("要素が今の表示に無ければ、座標へ退避せず orphan にする", () => {
    const out = locateWith(
      [{ id: "a", anchor: { kind: "point", x: 0.5, y: 0.1, selector: "#gone", excerpt: "x" } }],
      {},
    );
    expect(out.a?.orphan).toBe(true);
  });

  it("大きさの無い（display:none など）要素は orphan", () => {
    const out = locateWith(
      [{ id: "a", anchor: { kind: "point", selector: "#p", rx: 0.5, ry: 0.5 } }],
      { "#p": stubEl({ rect: { left: 0, top: 0, width: 0, height: 0 } }) },
    );
    expect(out.a?.orphan).toBe(true);
  });

  it("同じセレクタが別画面の別の要素に当たっても（抜粋が違えば）orphan", () => {
    const out = locateWith(
      [{ id: "a", anchor: { kind: "point", selector: "#p", rx: 0.5, ry: 0.5, excerpt: "ログイン画面のボタン" } }],
      { "#p": stubEl({ rect, text: "タスク一覧の見出し" }) },
    );
    expect(out.a?.orphan).toBe(true);
  });

  it("overflow の箱（パン・内側スクロールの外）に隠れた要素は null（ピンを出さない）", () => {
    const clipper = stubEl({ rect: { left: 0, top: 0, width: 800, height: 80 } });
    const el = stubEl({ rect, text: "本文", parent: clipper });
    const styles = new Map([[clipper, { overflowX: "hidden", overflowY: "hidden" }]]);
    const out = locateWith(
      [{ id: "a", anchor: { kind: "point", selector: "#p", rx: 0.5, ry: 0.5, excerpt: "本文" } }],
      { "#p": el },
      styles,
    );
    // 要素の中央 (200,150) は箱 (高さ 80) の外
    expect(out.a).toBeNull();
  });

  it("セレクタの無いコメントは文書の割合で置く（範囲は右上）", () => {
    const out = locateWith([{ id: "a", anchor: { kind: "region", x: 0.2, y: 0.1, w: 0.3 } }], {});
    expect(out.a).toEqual({ x: 500, y: 200, orphan: false });
  });
});
