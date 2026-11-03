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

/**
 * コメントの表示位置（ルート左上を原点とする px）。解決できなければ退避の割合位置を使い、
 * `orphan` を立てる（要素が書き換えで消えた印。表示は残す）。
 * ルートの表示領域の外（スクロールで隠れている）なら null。
 */
export function locateAnchor(
  root: Element,
  anchor: CommentAnchor,
): { x: number; y: number; orphan: boolean } | null {
  const rootBox = anchorBox(root);
  const el = resolveSelector(root, anchor.selector);
  if (el) {
    const r = el.getBoundingClientRect();
    if (r.width > 0 || r.height > 0) {
      const point =
        anchor.kind === "point"
          ? { x: r.left + clamp01(anchor.rx ?? 0.5) * r.width, y: r.top + clamp01(anchor.ry ?? 0.5) * r.height }
          : { x: r.right, y: r.top };
      const x = point.x - rootBox.left;
      const y = point.y - rootBox.top;
      if (x < 0 || y < 0 || x > rootBox.width || y > rootBox.height) return null;
      return { x, y, orphan: false };
    }
  }
  if (anchor.x === undefined || anchor.y === undefined) return null;
  const p = fromFraction({ x: anchor.x, y: anchor.y }, rootBox);
  return { ...p, orphan: !!anchor.selector };
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
  /** 各コメントの表示位置。隠れている・位置を持たないものは null */
  locate(
    comments: ArtifactComment[],
  ): Promise<Record<string, { x: number; y: number; orphan: boolean } | null>>;
  /** スクロール・リサイズ・内容変化で位置が変わりうるとき `cb` を呼ぶ。戻り値は購読解除 */
  subscribe(cb: () => void): () => void;
}
