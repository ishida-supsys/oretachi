import type { ArtifactComment, CommentAnchor } from "../types/artifactComment";

/**
 * コメントの位置まわりの純粋関数と、描画ルート内の DOM を相手にする拾い上げ・位置解決。
 * 純粋な部分だけ単体テストする（vitest は node 環境で DOM が無い）。
 */

export interface Point {
  x: number;
  y: number;
}

export interface Box {
  left: number;
  top: number;
  width: number;
  height: number;
}

/** ドラッグと見なす最小移動量（px）。これ未満はクリック（点コメント） */
export const DRAG_THRESHOLD_PX = 6;
/** 抜粋の最大文字数 */
export const EXCERPT_MAX_CHARS = 200;

export function clamp01(v: number): number {
  if (!Number.isFinite(v)) return 0;
  return Math.min(1, Math.max(0, v));
}

/** 2 点を結ぶ矩形。ドラッグの向きに依らず left/top が小さい側になる */
export function boxFromPoints(a: Point, b: Point): Box {
  return {
    left: Math.min(a.x, b.x),
    top: Math.min(a.y, b.y),
    width: Math.abs(a.x - b.x),
    height: Math.abs(a.y - b.y),
  };
}

export function isDrag(a: Point, b: Point): boolean {
  return Math.hypot(a.x - b.x, a.y - b.y) >= DRAG_THRESHOLD_PX;
}

/** 描画ルート内の px 位置を割合へ。ルートが潰れていても NaN を出さない */
export function toFraction(p: Point, root: Box): Point {
  return {
    x: root.width > 0 ? clamp01((p.x - root.left) / root.width) : 0,
    y: root.height > 0 ? clamp01((p.y - root.top) / root.height) : 0,
  };
}

/** 割合を描画ルート内の px へ（ルート左上が原点） */
export function fromFraction(f: Point, root: Box): Point {
  return { x: clamp01(f.x) * root.width, y: clamp01(f.y) * root.height };
}

/** 空白を畳んで切り詰めた抜粋 */
export function makeExcerpt(text: string | null | undefined): string {
  const flat = (text ?? "").replace(/\s+/g, " ").trim();
  if (flat.length <= EXCERPT_MAX_CHARS) return flat;
  return `${flat.slice(0, EXCERPT_MAX_CHARS)}…`;
}

/**
 * アンカーが位置の手がかり（セレクタ、または割合の座標）を持つか。
 * AI が付けるコメントは `{ excerpt }` だけ・anchor 無しのこともあり、それは位置を持たない。
 * 位置を持たないものはピンにできないので、層の端に並べて出す（見えなくならないように）。
 */
export function hasAnchorPosition(anchor: CommentAnchor | undefined): boolean {
  if (!anchor) return false;
  if (typeof anchor.selector === "string" && anchor.selector.length > 0) return true;
  return typeof anchor.x === "number" && typeof anchor.y === "number";
}

/**
 * アプリのウィンドウごとに見せるコメントを決める。
 * サブウィンドウの ID は起動のたびに採番し直すので、閉じて開き直すと旧 ID のコメントの持ち主がいなくなる。
 * そのため常駐ウィンドウ（"main"）には、他ウィンドウ宛のコメントも位置なしで出す
 * （他ウィンドウのセレクタを main の DOM へ当てるとずれた場所に出るため、anchor は抜粋だけにする。
 * 返信・解決・削除は ID で行うので影響しない）。サブウィンドウには自分宛のものだけを出す。
 */
export function commentsForWindow(all: ArtifactComment[], windowId: string): ArtifactComment[] {
  if (windowId !== "main") return all.filter((c) => c.anchor?.window === windowId);
  return all.map((c) => {
    const w = c.anchor?.window ?? "main";
    if (w === "main") return c;
    return { ...c, anchor: { excerpt: c.anchor?.excerpt, window: w } };
  });
}

/** 表示用の並び（付けた順）と未解決だけの絞り込み */
export function visibleComments(list: ArtifactComment[] | undefined): ArtifactComment[] {
  return (list ?? [])
    .filter((c) => !c.resolved)
    .slice()
    .sort((a, b) => a.createdAt - b.createdAt);
}

/** 解決済みでないコメントの件数 */
export function unresolvedCount(list: ArtifactComment[] | undefined): number {
  return (list ?? []).filter((c) => !c.resolved).length;
}

/** ポップアップを `anchor` の隣に置く位置。ルートの外へはみ出さないよう左右・上下を寄せる */
export function placePopup(anchor: Point, popup: { width: number; height: number }, root: { width: number; height: number }): Point {
  const gap = 8;
  let x = anchor.x + gap;
  if (x + popup.width > root.width) x = anchor.x - gap - popup.width;
  let y = anchor.y;
  if (y + popup.height > root.height) y = root.height - popup.height;
  return {
    x: Math.max(0, Math.min(x, Math.max(0, root.width - popup.width))),
    y: Math.max(0, y),
  };
}

// ─── DOM（描画ルート内の要素を相手にする） ─────────────────────────────────

/**
 * `root` からの CSS パスを作る。`#id` があり、ルート内で一意ならそこで止める。
 * 返す文字列は `root.querySelector` に渡せる（先頭は `:scope >`）。
 */
export function buildSelector(el: Element, root: Element): string | null {
  if (el === root || !root.contains(el)) return null;
  const parts: string[] = [];
  let cur: Element | null = el;
  while (cur && cur !== root) {
    const id = cur.id;
    if (id && /^[A-Za-z][\w-]*$/.test(id) && root.querySelectorAll(`#${id}`).length === 1) {
      parts.unshift(`#${id}`);
      return parts.join(" > ");
    }
    const parent: Element | null = cur.parentElement;
    if (!parent) return null;
    const tag = cur.tagName.toLowerCase();
    const sameTag = Array.from(parent.children).filter((c) => c.tagName === cur!.tagName);
    parts.unshift(sameTag.length > 1 ? `${tag}:nth-of-type(${sameTag.indexOf(cur) + 1})` : tag);
    cur = parent;
  }
  return `:scope > ${parts.join(" > ")}`;
}

/** 解決できなければ null（不正なセレクタでも例外にしない） */
export function resolveSelector(root: Element, selector: string | undefined): Element | null {
  if (!selector) return null;
  try {
    return root.querySelector(selector);
  } catch {
    return null;
  }
}

/** 2 つの要素を含む最も内側の要素（ドラッグ範囲の両端から「範囲を覆う要素」を決める） */
export function commonAncestor(a: Element, b: Element, root: Element): Element {
  let cur: Element | null = a;
  while (cur && cur !== root) {
    if (cur.contains(b)) return cur;
    cur = cur.parentElement;
  }
  return root;
}

export function anchorBox(root: Element): Box {
  const r = root.getBoundingClientRect();
  return { left: r.left, top: r.top, width: r.width, height: r.height };
}

/** 抜粋の突き合わせに使う先頭の文字数 */
export const EXCERPT_MATCH_CHARS = 40;

/**
 * 保存した抜粋と、いまの要素のテキストが同じ要素を指しているとみなせるか。
 * ページ切り替えのある SPA では、別の画面の同じ構造の要素に同じセレクタが当たってしまうので、
 * 内容で確かめる。抜粋が空（画像・図形など）なら確かめようがないので一致扱い。
 * 先頭だけ比べるのは、要素のテキストが後ろへ伸びる（件数・時刻の表示など）程度の変化を許すため。
 */
export function excerptMatches(stored: string | undefined, current: string | null | undefined): boolean {
  if (!stored) return true;
  const cur = makeExcerpt(current);
  const n = Math.min(EXCERPT_MATCH_CHARS, stored.length);
  return cur.slice(0, n) === stored.slice(0, n);
}

/**
 * 要素の矩形（ビューポート座標）に対するピンの位置。
 * `rx`/`ry` があればそこ（矩形の 1px 内側へ収める）。旧形式は point = 中央、それ以外 = 右上。
 */
export function pinPointInRect(anchor: CommentAnchor, r: Box): Point {
  const hasRel = typeof anchor.rx === "number" || typeof anchor.ry === "number";
  const rx = hasRel ? clamp01(anchor.rx ?? 0) : anchor.kind === "point" ? 0.5 : 1;
  const ry = hasRel ? clamp01(anchor.ry ?? 0) : anchor.kind === "point" ? 0.5 : 0;
  const x = r.left + rx * r.width;
  const y = r.top + ry * r.height;
  return {
    x: r.width > 1 ? Math.min(Math.max(x, r.left + 1), r.left + r.width - 1) : x,
    y: r.height > 1 ? Math.min(Math.max(y, r.top + 1), r.top + r.height - 1) : y,
  };
}

/** 点が矩形の中にあるか（端を含む） */
export function pointInBox(p: Point, b: Box): boolean {
  return p.x >= b.left && p.y >= b.top && p.x <= b.left + b.width && p.y <= b.top + b.height;
}

/** セレクタを持たないコメントの退避位置（文書・ルートに対する割合から）。範囲は右上 */
export function fractionPin(anchor: CommentAnchor, size: { width: number; height: number }): Point | null {
  if (typeof anchor.x !== "number" || typeof anchor.y !== "number") return null;
  const w = anchor.kind === "point" ? 0 : (anchor.w ?? 0);
  return { x: clamp01(anchor.x + w) * size.width, y: clamp01(anchor.y) * size.height };
}

/** 位置の解決結果。`orphan` = 指していた要素が今の表示に無い（別ページ・書き換え） */
export type Located = { x: number; y: number; orphan: boolean } | null;

/** 要素の祖先（root の手前まで）に `overflow` で切り取る箱があり、点がその外なら true */
function clippedByAncestor(el: Element, root: Element, p: Point): boolean {
  for (let a = el.parentElement; a && a !== root; a = a.parentElement) {
    const cs = getComputedStyle(a);
    if (cs.overflowX === "visible" && cs.overflowY === "visible") continue;
    const r = a.getBoundingClientRect();
    if (!pointInBox(p, { left: r.left, top: r.top, width: r.width, height: r.height })) return true;
  }
  return false;
}

/**
 * コメントの表示位置（ルート左上を原点とする px）。
 * - セレクタを持つ: 要素が見つからない・大きさが無い・抜粋が合わないなら `orphan`（今の表示には無い）。
 *   座標へは退避しない（別ページの同じ位置にピンが出てしまう）。
 * - 見つかっても、スクロール・パンで隠れている（ルートの外、または overflow の箱の外）なら null。
 * - セレクタを持たない: 割合の座標。
 */
export function locateAnchor(root: Element, anchor: CommentAnchor): Located {
  const rootBox = anchorBox(root);
  if (anchor.selector) {
    const el = resolveSelector(root, anchor.selector);
    const r = el?.getBoundingClientRect();
    if (!el || !r || (r.width <= 0 && r.height <= 0) || !excerptMatches(anchor.excerpt, el.textContent)) {
      return { x: 0, y: 0, orphan: true };
    }
    const p = pinPointInRect(anchor, { left: r.left, top: r.top, width: r.width, height: r.height });
    if (!pointInBox(p, { left: rootBox.left, top: rootBox.top, width: rootBox.width, height: rootBox.height })) return null;
    if (clippedByAncestor(el, root, p)) return null;
    return { x: p.x - rootBox.left, y: p.y - rootBox.top, orphan: false };
  }
  const f = fractionPin(anchor, rootBox);
  return f ? { ...f, orphan: false } : null;
}

// ─── 描画方式ごとの差を吸収するアダプタ ─────────────────────────────────────

/** 拾い上げの結果。そのままコメントの anchor になる */
export type PickedAnchor = CommentAnchor;

/**
 * コメント位置の拾い上げ・位置解決を、描画方式（同一文書の DOM / sandbox iframe / アプリのウィンドウ）
 * ごとに差し替えるためのアダプタ。座標はすべて描画ルート左上を原点とする px。
 */
export interface CommentAdapter {
  /** `b` が null ならクリック（点）、あれば `a`〜`b` のドラッグ。拾えなければ null */
  pick(a: Point, b: Point | null): Promise<PickedAnchor | null>;
  /** 各コメントの表示位置。隠れている（スクロール・パンの外）・位置を持たないものは null。今の表示に無いものは orphan */
  locate(comments: ArtifactComment[]): Promise<Record<string, Located>>;
  /** スクロール・リサイズ・内容変化で位置が変わりうるとき `cb` を呼ぶ。戻り値は購読解除 */
  subscribe(cb: () => void): () => void;
}
