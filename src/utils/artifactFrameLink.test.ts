/**
 * iframe 内で動くリンク横取りスクリプト（`ARTIFACT_LINK_INTERCEPT_JS`）と、
 * 親側の読み取り（`readArtifactLinkHoverMessage`）の振る舞いを確認する。
 *
 * スクリプトの実体は srcdoc へ埋め込む文字列なので、artifactBridgeJs.test.ts と同じく
 * `new Function` で必要なグローバル（document / window / parent）だけ束縛して評価する。
 */
import { describe, it, expect, beforeEach } from "vitest";
import {
  ARTIFACT_LINK_INTERCEPT_JS,
  ARTIFACT_LINK_HOVER_MARKER,
  ARTIFACT_NAVIGATE_MARKER,
  readArtifactLinkHoverMessage,
} from "./artifactFrameLink";

type Listener = (event: unknown) => void;

interface FakeAnchor {
  nodeType: number;
  tagName: string;
  hasAttribute(name: string): boolean;
  getAttribute(name: string): string | null;
  getBoundingClientRect(): { left: number; top: number; width: number; height: number };
}

function anchor(href: string | null, rect = { left: 10, top: 20, width: 100, height: 16 }): FakeAnchor {
  return {
    nodeType: 1,
    tagName: "A",
    hasAttribute: (name) => name === "href" && href !== null,
    getAttribute: (name) => (name === "href" ? href : null),
    getBoundingClientRect: () => rect,
  };
}

/** composedPath を持つ最小のイベント。preventDefault / stopPropagation の呼ばれ方も見る */
function event(path: unknown[]) {
  let prevented = false;
  let stopped = false;
  return {
    composedPath: () => path,
    preventDefault: () => { prevented = true; },
    stopPropagation: () => { stopped = true; },
    get prevented() { return prevented; },
    get stopped() { return stopped; },
  };
}

function setup() {
  const posted: Record<string, unknown>[] = [];
  const docListeners = new Map<string, Listener[]>();
  const winListeners = new Map<string, Listener[]>();

  function add(map: Map<string, Listener[]>) {
    return (type: string, fn: Listener) => {
      const list = map.get(type) ?? [];
      list.push(fn);
      map.set(type, list);
    };
  }

  const doc = { addEventListener: add(docListeners) };
  const win = { addEventListener: add(winListeners) };
  const parent = { postMessage: (msg: Record<string, unknown>) => { posted.push(msg); } };

  new Function("document", "window", "parent", ARTIFACT_LINK_INTERCEPT_JS)(doc, win, parent);

  function fire(map: Map<string, Listener[]>, type: string, e: unknown) {
    (map.get(type) ?? []).forEach((fn) => fn(e));
  }

  return {
    posted,
    onDocument: (type: string, e: unknown) => fire(docListeners, type, e),
    onWindow: (type: string, e: unknown) => fire(winListeners, type, e),
  };
}

describe("ARTIFACT_LINK_INTERCEPT_JS: ホバー通知", () => {
  let ctx: ReturnType<typeof setup>;

  beforeEach(() => {
    ctx = setup();
  });

  it("リンクへ入ると href と座標を親へ送る", () => {
    ctx.onDocument("mouseover", event([anchor("https://example.com/a")]));
    expect(ctx.posted).toEqual([
      {
        [ARTIFACT_LINK_HOVER_MARKER]: true,
        href: "https://example.com/a",
        rect: { left: 10, top: 20, width: 100, height: 16 },
      },
    ]);
  });

  it("href は生値を trim して送る（表示・コピー対象がそのまま URL になる）", () => {
    ctx.onDocument("mouseover", event([anchor("  artifact:abc  ")]));
    expect(ctx.posted[0]?.href).toBe("artifact:abc");
  });

  it("リンクから出ると閉じる要求を送る", () => {
    ctx.onDocument("mouseover", event([anchor("https://example.com/a")]));
    ctx.onDocument("mouseout", event([anchor("https://example.com/a")]));
    expect(ctx.posted[1]).toEqual({
      [ARTIFACT_LINK_HOVER_MARKER]: true,
      href: null,
      rect: null,
    });
  });

  it("ページ内アンカー（#...）と href 空は出さない", () => {
    ctx.onDocument("mouseover", event([anchor("#heading")]));
    ctx.onDocument("mouseover", event([anchor("   ")]));
    expect(ctx.posted).toEqual([]);
  });

  it("リンク以外へ入ると閉じるが、出しっぱなしでなければ何も送らない", () => {
    ctx.onDocument("mouseover", event([{ nodeType: 1, tagName: "DIV" }]));
    expect(ctx.posted).toEqual([]);

    ctx.onDocument("mouseover", event([anchor("https://example.com/a")]));
    ctx.onDocument("mouseover", event([{ nodeType: 1, tagName: "DIV" }]));
    expect(ctx.posted).toHaveLength(2);
    expect(ctx.posted[1]?.href).toBeNull();
    // 閉じたあとは重ねて送らない
    ctx.onDocument("mouseover", event([{ nodeType: 1, tagName: "DIV" }]));
    expect(ctx.posted).toHaveLength(2);
  });

  it("スクロール / フォーカス喪失でも閉じる（座標が合わなくなる）", () => {
    ctx.onDocument("mouseover", event([anchor("https://example.com/a")]));
    ctx.onDocument("scroll", event([]));
    expect(ctx.posted[1]?.href).toBeNull();

    ctx.onDocument("mouseover", event([anchor("https://example.com/a")]));
    ctx.onWindow("blur", event([]));
    expect(ctx.posted[3]?.href).toBeNull();
  });

  it("artifact: リンクのクリック横取りは従来どおり動く", () => {
    const e = event([anchor("artifact://worktree/w1/a1")]);
    ctx.onDocument("click", e);
    expect(e.prevented).toBe(true);
    expect(ctx.posted).toEqual([
      { [ARTIFACT_NAVIGATE_MARKER]: true, href: "artifact://worktree/w1/a1" },
    ]);
  });
});

describe("ARTIFACT_LINK_INTERCEPT_JS: クリックの不発化", () => {
  let ctx: ReturnType<typeof setup>;

  beforeEach(() => {
    ctx = setup();
  });

  // sandbox は遷移を塞ぐが、塞いだ結果 iframe の表示が壊れる。ここで先に止める
  it("外部 URL のクリックは不発にし、親へも何も送らない", () => {
    for (const type of ["click", "auxclick"]) {
      const e = event([anchor("https://example.com/a")]);
      ctx.onDocument(type, e);
      expect(e.prevented).toBe(true);
      // 伝播は止めない。止めるとアーティファクト自身の onClick / デリゲーションが死ぬ
      expect(e.stopped).toBe(false);
    }
    expect(ctx.posted).toEqual([]);
  });

  it("http(s) 以外・相対パス・空 href も不発にする", () => {
    for (const href of ["mailto:a@example.com", "./other.html", "", "javascript:alert(1)"]) {
      const e = event([anchor(href)]);
      ctx.onDocument("click", e);
      expect(e.prevented).toBe(true);
    }
    expect(ctx.posted).toEqual([]);
  });

  it("ページ内アンカー（#...）は素通しする（同じ文書内のジャンプ）", () => {
    const e = event([anchor("#heading")]);
    ctx.onDocument("click", e);
    expect(e.prevented).toBe(false);
    expect(ctx.posted).toEqual([]);
  });

  it("リンク以外のクリックには触らない", () => {
    const e = event([{ nodeType: 1, tagName: "DIV" }]);
    ctx.onDocument("click", e);
    expect(e.prevented).toBe(false);
  });
});

describe("readArtifactLinkHoverMessage", () => {
  const frame = { contentWindow: {} } as unknown as HTMLIFrameElement;
  const from = (source: unknown, data: unknown) =>
    ({ source, data }) as unknown as MessageEvent;

  const validRect = { left: 1, top: 2, width: 3, height: 4 };
  const show = { [ARTIFACT_LINK_HOVER_MARKER]: true, href: "https://example.com", rect: validRect };

  it("対象 iframe から来た通知だけ受ける", () => {
    expect(readArtifactLinkHoverMessage(from(frame.contentWindow, show), frame)).toEqual({
      href: "https://example.com",
      rect: validRect,
    });
    expect(readArtifactLinkHoverMessage(from({}, show), frame)).toBeNull();
    expect(readArtifactLinkHoverMessage(from(frame.contentWindow, show), null)).toBeNull();
  });

  it("マーカーが無いメッセージは無視する", () => {
    expect(readArtifactLinkHoverMessage(from(frame.contentWindow, { href: "x" }), frame)).toBeNull();
    expect(readArtifactLinkHoverMessage(from(frame.contentWindow, null), frame)).toBeNull();
    expect(readArtifactLinkHoverMessage(from(frame.contentWindow, "hover"), frame)).toBeNull();
  });

  it("href が null なら閉じる要求として返す", () => {
    const hide = { [ARTIFACT_LINK_HOVER_MARKER]: true, href: null, rect: null };
    expect(readArtifactLinkHoverMessage(from(frame.contentWindow, hide), frame)).toEqual({
      href: null,
      rect: null,
    });
  });

  it("href が文字列でなければ捨てる", () => {
    const bad = { [ARTIFACT_LINK_HOVER_MARKER]: true, href: 42, rect: validRect };
    expect(readArtifactLinkHoverMessage(from(frame.contentWindow, bad), frame)).toBeNull();
  });

  it("座標が壊れている通知は閉じる要求に落とす（変な位置に出さない）", () => {
    for (const rect of [undefined, null, "1,2", { left: 1, top: 2 }, { ...validRect, top: NaN }]) {
      expect(
        readArtifactLinkHoverMessage(
          from(frame.contentWindow, { [ARTIFACT_LINK_HOVER_MARKER]: true, href: "https://e.com", rect }),
          frame,
        ),
      ).toEqual({ href: null, rect: null });
    }
  });
});
