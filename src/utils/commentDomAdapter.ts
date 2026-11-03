import type { ArtifactComment } from "../types/artifactComment";
import {
  anchorBox,
  boxFromPoints,
  buildSelector,
  clamp01,
  commonAncestor,
  locateAnchor,
  makeExcerpt,
  toFraction,
  type CommentAdapter,
  type PickedAnchor,
  type Point,
} from "./commentAnchor";

/**
 * 同一文書の DOM（markdown / コード / SVG / mermaid / テーブルなど、親ドキュメントに直接描画するビュー）用。
 * `root` は描画ルート、`isLayer` はコメント層自身（拾い上げの対象から外す）の判定。
 */
export function createDomCommentAdapter(
  root: HTMLElement,
  isLayer: (el: Element) => boolean,
): CommentAdapter {
  function hit(p: Point): Element | null {
    const box = anchorBox(root);
    const stack = document.elementsFromPoint(box.left + p.x, box.top + p.y);
    for (const el of stack) {
      if (isLayer(el)) continue;
      if (el === root || !root.contains(el)) continue;
      return el;
    }
    return null;
  }

  return {
    async pick(a, b) {
      const box = anchorBox(root);
      const endPoint = b ?? a;
      const frac = toFraction({ x: box.left + endPoint.x, y: box.top + endPoint.y }, box);
      const elA = hit(a);
      if (!b) {
        if (!elA) return { kind: "point", x: frac.x, y: frac.y };
        const r = elA.getBoundingClientRect();
        return {
          kind: "point",
          x: frac.x,
          y: frac.y,
          selector: buildSelector(elA, root) ?? undefined,
          rx: r.width > 0 ? clamp01((box.left + a.x - r.left) / r.width) : 0.5,
          ry: r.height > 0 ? clamp01((box.top + a.y - r.top) / r.height) : 0.5,
          excerpt: makeExcerpt(elA.textContent),
        };
      }
      // ドラッグ: 両端の要素を含む最も内側の要素を対象にする
      const elB = hit(b);
      const target = elA && elB ? commonAncestor(elA, elB, root) : (elA ?? elB);
      const drag = boxFromPoints(a, b);
      const topLeft = toFraction({ x: box.left + drag.left, y: box.top + drag.top }, box);
      const result: PickedAnchor = {
        kind: target && target !== root ? "element" : "region",
        x: topLeft.x,
        y: topLeft.y,
        w: box.width > 0 ? clamp01(drag.width / box.width) : 0,
        h: box.height > 0 ? clamp01(drag.height / box.height) : 0,
      };
      if (target && target !== root) {
        result.selector = buildSelector(target, root) ?? undefined;
        result.excerpt = makeExcerpt(target.textContent);
        // ピンはドラッグした範囲の右上に置く（共通祖先は範囲よりずっと大きいことがある）
        const r = target.getBoundingClientRect();
        if (r.width > 0 && r.height > 0) {
          result.rx = clamp01((box.left + drag.left + drag.width - r.left) / r.width);
          result.ry = clamp01((box.top + drag.top - r.top) / r.height);
          result.rw = clamp01(drag.width / r.width);
          result.rh = clamp01(drag.height / r.height);
        }
      }
      return result;
    },

    async locate(comments: ArtifactComment[]) {
      const out: Record<string, { x: number; y: number; orphan: boolean } | null> = {};
      for (const c of comments) out[c.id] = locateAnchor(root, c.anchor ?? {});
      return out;
    },

    subscribe(cb) {
      let raf = 0;
      const schedule = () => {
        if (raf) return;
        raf = requestAnimationFrame(() => {
          raf = 0;
          cb();
        });
      };
      // 内側のスクロール容器（コード・テーブルなど）のスクロールも拾うため capture で受ける
      root.addEventListener("scroll", schedule, true);
      const ro = new ResizeObserver(schedule);
      ro.observe(root);
      const mo = new MutationObserver((records) => {
        // コメント層自身の描画（ピン・ポップアップ）の変化では再計算しない（自分で自分を起こさない）
        const own = records.every((r) => {
          const el = r.target instanceof Element ? r.target : r.target.parentElement;
          return !!el && isLayer(el);
        });
        if (!own) schedule();
      });
      mo.observe(root, { subtree: true, childList: true, attributes: true, characterData: true });
      window.addEventListener("resize", schedule);
      return () => {
        if (raf) cancelAnimationFrame(raf);
        root.removeEventListener("scroll", schedule, true);
        ro.disconnect();
        mo.disconnect();
        window.removeEventListener("resize", schedule);
      };
    },
  };
}
