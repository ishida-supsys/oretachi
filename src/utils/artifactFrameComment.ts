/**
 * sandbox iframe（html / react アーティファクト）の中身にコメントを付けるためのブリッジ。
 *
 * iframe は `sandbox="allow-scripts"`（allow-same-origin なし）で、親は中の DOM に触れない。
 * そこで iframe の中に小さなスクリプト（`ARTIFACT_COMMENT_PROBE_JS`）を注入し、親からの
 * 「この座標の要素は？」「このコメントはいま画面のどこ？」という問い合わせに答えさせる。
 * 注入は `artifactFrameLink` のリンク横取りと同じ経路（srcdoc の inline script）で、
 * 追加の CSP 緩和は要らない。
 *
 * 信頼境界: アーティファクトのコードは AI 生成で、同じ iframe の中にいる。返事は偽造されうるので、
 * 親は中身を `sanitizeFrameAnchor` / `sanitizeFramePositions` で形・長さ・型を検めてから使い、
 * 受け取った値は位置の情報としてしか扱わない（権限の判断には使わない）。
 * 送信元は `event.source === iframe.contentWindow` で判定する（opaque origin なので origin は使えない）。
 *
 * 位置の保存はドキュメント基準（スクロール量に依存しない割合）。ビューポート基準にすると
 * スクロールしただけでコメントがずれる。
 */

import type { CommentAnchor } from "../types/artifactComment";
import type { CommentAdapter, PickedAnchor, Point } from "./commentAnchor";
import { EXCERPT_MAX_CHARS } from "./commentAnchor";

/** 親 → iframe の問い合わせ */
export const ARTIFACT_COMMENT_PROBE_MARKER = "__oretachiCommentProbe";
/** iframe → 親の返事 */
export const ARTIFACT_COMMENT_PROBE_RESULT_MARKER = "__oretachiCommentProbeResult";
/** iframe → 親: スクロール・リサイズ・内容変化で位置が変わった（再問い合わせの合図） */
export const ARTIFACT_COMMENT_PROBE_CHANGED_MARKER = "__oretachiCommentProbeChanged";

/** 返事を待つ上限（ms）。iframe が固まっていても親の UI を止めない */
const PROBE_TIMEOUT_MS = 1500;

/**
 * iframe 内に注入するスクリプト（ES5 の文字列。SFC パーサーとの干渉を避けるため SFC の外に置く）。
 * 問い合わせは `{ [REQ]: true, id, op: "pick" | "locate", ... }`。座標は iframe のビューポート px。
 */
export const ARTIFACT_COMMENT_PROBE_JS =
  "(function(){" +
  "  var REQ=" + JSON.stringify(ARTIFACT_COMMENT_PROBE_MARKER) + ";" +
  "  var RES=" + JSON.stringify(ARTIFACT_COMMENT_PROBE_RESULT_MARKER) + ";" +
  "  var CHG=" + JSON.stringify(ARTIFACT_COMMENT_PROBE_CHANGED_MARKER) + ";" +
  "  var MAX=" + EXCERPT_MAX_CHARS + ";" +
  "  function root(){return document.body||document.documentElement;}" +
  "  function post(msg){try{parent.postMessage(msg,'*');}catch(err){}}" +
  "  function clamp(v){return v!==v?0:(v<0?0:(v>1?1:v));}" +
  "  function docSize(){" +
  "    var d=document.documentElement,b=document.body||d;" +
  "    return {w:Math.max(d.scrollWidth,b.scrollWidth,window.innerWidth,1)," +
  "            h:Math.max(d.scrollHeight,b.scrollHeight,window.innerHeight,1)};" +
  "  }" +
  "  function excerpt(t){" +
  "    t=String(t||'').replace(/\\s+/g,' ').trim();" +
  "    return t.length>MAX?t.slice(0,MAX)+'\\u2026':t;" +
  "  }" +
  "  function selectorOf(el){" +
  "    var r=root();" +
  "    if(el===r||!r.contains(el))return null;" +
  "    var parts=[],cur=el;" +
  "    while(cur&&cur!==r){" +
  "      var id=cur.id;" +
  "      if(id&&/^[A-Za-z][\\w-]*$/.test(id)&&r.querySelectorAll('#'+id).length===1){" +
  "        parts.unshift('#'+id);return parts.join(' > ');" +
  "      }" +
  "      var p=cur.parentElement;" +
  "      if(!p)return null;" +
  "      var same=[],i;" +
  "      for(i=0;i<p.children.length;i++){if(p.children[i].localName===cur.localName)same.push(p.children[i]);}" +
  "      parts.unshift(same.length>1?cur.localName+':nth-of-type('+(same.indexOf(cur)+1)+')':cur.localName);" +
  "      cur=p;" +
  "    }" +
  "    return ':scope > '+parts.join(' > ');" +
  "  }" +
  "  function hit(x,y){" +
  "    var r=root(),list=document.elementsFromPoint(x,y),i;" +
  "    for(i=0;i<list.length;i++){" +
  "      if(list[i]!==r&&list[i]!==document.documentElement&&r.contains(list[i]))return list[i];" +
  "    }" +
  "    return null;" +
  "  }" +
  "  function common(a,b){" +
  "    var r=root(),cur=a;" +
  "    while(cur&&cur!==r){if(cur.contains(b))return cur;cur=cur.parentElement;}" +
  "    return null;" +
  "  }" +
  "  function pick(d){" +
  "    var ds=docSize(),sx=window.scrollX,sy=window.scrollY,ea=hit(d.x,d.y),a,t;" +
  "    if(d.x2===null||d.x2===undefined){" +
  "      a={kind:'point',x:clamp((d.x+sx)/ds.w),y:clamp((d.y+sy)/ds.h)};" +
  "      if(ea){" +
  "        var r=ea.getBoundingClientRect(),s=selectorOf(ea);" +
  "        if(s)a.selector=s;" +
  "        a.rx=r.width>0?clamp((d.x-r.left)/r.width):0.5;" +
  "        a.ry=r.height>0?clamp((d.y-r.top)/r.height):0.5;" +
  "        a.excerpt=excerpt(ea.textContent);" +
  "      }" +
  "      return a;" +
  "    }" +
  "    var eb=hit(d.x2,d.y2);" +
  "    t=(ea&&eb)?common(ea,eb):(ea||eb);" +
  "    a={kind:t?'element':'region'," +
  "       x:clamp((Math.min(d.x,d.x2)+sx)/ds.w),y:clamp((Math.min(d.y,d.y2)+sy)/ds.h)," +
  "       w:clamp(Math.abs(d.x-d.x2)/ds.w),h:clamp(Math.abs(d.y-d.y2)/ds.h)};" +
  "    if(t){var s2=selectorOf(t);if(s2)a.selector=s2;a.excerpt=excerpt(t.textContent);}" +
  "    return a;" +
  "  }" +
  "  function locateOne(a){" +
  "    if(!a||typeof a!=='object')return null;" +
  "    var vw=window.innerWidth,vh=window.innerHeight,px,py,orphan=false,el=null;" +
  "    if(typeof a.selector==='string'&&a.selector){" +
  "      try{el=root().querySelector(a.selector);}catch(err){el=null;}" +
  "    }" +
  "    if(el){" +
  "      var r=el.getBoundingClientRect();" +
  "      if(r.width>0||r.height>0){" +
  "        if(a.kind==='point'){px=r.left+clamp(a.rx===undefined?0.5:a.rx)*r.width;py=r.top+clamp(a.ry===undefined?0.5:a.ry)*r.height;}" +
  "        else{px=r.right;py=r.top;}" +
  "      }else{el=null;}" +
  "    }" +
  "    if(!el){" +
  "      if(typeof a.x!=='number'||typeof a.y!=='number')return null;" +
  "      var ds=docSize();" +
  "      px=clamp(a.x)*ds.w-window.scrollX;py=clamp(a.y)*ds.h-window.scrollY;" +
  "      orphan=!!a.selector;" +
  "    }" +
  "    if(px<0||py<0||px>vw||py>vh)return null;" +
  "    return {x:px,y:py,orphan:orphan};" +
  "  }" +
  "  window.addEventListener('message',function(e){" +
  "    if(e.source!==parent)return;" +
  "    var d=e.data;" +
  "    if(!d||typeof d!=='object'||d[REQ]!==true)return;" +
  "    var out={id:d.id};" +
  "    try{" +
  "      if(d.op==='pick'){out.anchor=pick(d);}" +
  "      else if(d.op==='locate'){" +
  "        out.positions={};" +
  "        var items=Array.isArray(d.items)?d.items:[],i;" +
  "        for(i=0;i<items.length;i++){if(items[i]&&typeof items[i].id==='string')out.positions[items[i].id]=locateOne(items[i].anchor);}" +
  "      }" +
  "    }catch(err){}" +
  "    var msg={};msg[RES]=true;msg.id=out.id;msg.anchor=out.anchor;msg.positions=out.positions;" +
  "    post(msg);" +
  "  });" +
  // 位置が動きうる変化は rAF でまとめて 1 回だけ親へ知らせる
  "  var raf=0;" +
  "  function changed(){" +
  "    if(raf)return;" +
  "    raf=requestAnimationFrame(function(){raf=0;var m={};m[CHG]=true;post(m);});" +
  "  }" +
  "  document.addEventListener('scroll',changed,true);" +
  "  window.addEventListener('resize',changed);" +
  "  window.addEventListener('load',changed);" +
  "  try{new MutationObserver(changed).observe(document.documentElement," +
  "    {subtree:true,childList:true,attributes:true,characterData:true});}catch(err){}" +
  "})();";

// ─── 親側: 返事の検査 ──────────────────────────────────────────────────────

function finite(v: unknown): v is number {
  return typeof v === "number" && Number.isFinite(v);
}

function frac(v: unknown): number | undefined {
  return finite(v) ? Math.min(1, Math.max(0, v)) : undefined;
}

function str(v: unknown, max: number): string | undefined {
  return typeof v === "string" && v.length > 0 ? v.slice(0, max) : undefined;
}

const KINDS = new Set(["point", "element", "region"]);

/** iframe から来た anchor を、既知のキー・型・長さへ絞る（偽造・肥大化への備え） */
export function sanitizeFrameAnchor(raw: unknown): PickedAnchor | null {
  if (!raw || typeof raw !== "object") return null;
  const r = raw as Record<string, unknown>;
  const out: CommentAnchor = {};
  if (typeof r.kind === "string" && KINDS.has(r.kind)) out.kind = r.kind as CommentAnchor["kind"];
  for (const key of ["x", "y", "w", "h", "rx", "ry"] as const) {
    const v = frac(r[key]);
    if (v !== undefined) out[key] = v;
  }
  const selector = str(r.selector, 1000);
  if (selector) out.selector = selector;
  const excerpt = str(r.excerpt, EXCERPT_MAX_CHARS + 1);
  if (excerpt) out.excerpt = excerpt;
  return Object.keys(out).length > 0 ? out : null;
}

export type FramePosition = { x: number; y: number; orphan: boolean } | null;

/** `locate` の返事を `{ commentId: 位置 }` へ絞る。要求していない ID は捨てる */
export function sanitizeFramePositions(
  raw: unknown,
  wantedIds: readonly string[],
): Record<string, FramePosition> {
  const out: Record<string, FramePosition> = {};
  const src = raw && typeof raw === "object" ? (raw as Record<string, unknown>) : {};
  for (const id of wantedIds) {
    const p = Object.prototype.hasOwnProperty.call(src, id) ? src[id] : null;
    if (p && typeof p === "object") {
      const { x, y, orphan } = p as Record<string, unknown>;
      out[id] = finite(x) && finite(y) ? { x, y, orphan: orphan === true } : null;
    } else {
      out[id] = null;
    }
  }
  return out;
}

// ─── 親側: アダプタ ────────────────────────────────────────────────────────

interface ProbeResult {
  anchor?: unknown;
  positions?: unknown;
}

/**
 * sandbox iframe 用のアダプタ。`getFrame` は対象の iframe、`getRoot` は描画ルート
 * （コメント層の親。iframe はこの中のどこかにある）。
 */
export function createFrameCommentAdapter(
  getFrame: () => HTMLIFrameElement | null,
  getRoot: () => HTMLElement | null,
): CommentAdapter {
  let seq = 0;
  const pending = new Map<number, (r: ProbeResult) => void>();

  function onMessage(event: MessageEvent) {
    const frame = getFrame();
    if (!frame || event.source !== frame.contentWindow) return;
    const data = event.data as Record<string, unknown> | null;
    if (!data || typeof data !== "object" || data[ARTIFACT_COMMENT_PROBE_RESULT_MARKER] !== true) return;
    if (typeof data.id !== "number") return;
    const done = pending.get(data.id);
    if (!done) return;
    pending.delete(data.id);
    done({ anchor: data.anchor, positions: data.positions });
  }

  function ask(message: Record<string, unknown>): Promise<ProbeResult | null> {
    const frame = getFrame();
    if (!frame?.contentWindow) return Promise.resolve(null);
    const id = ++seq;
    return new Promise((resolve) => {
      const timer = setTimeout(() => {
        pending.delete(id);
        resolve(null);
      }, PROBE_TIMEOUT_MS);
      pending.set(id, (r) => {
        clearTimeout(timer);
        resolve(r);
      });
      frame.contentWindow!.postMessage({ [ARTIFACT_COMMENT_PROBE_MARKER]: true, id, ...message }, "*");
    });
  }

  /** iframe の左上を、描画ルート左上を原点として測った位置 */
  function frameOffset(): Point | null {
    const frame = getFrame();
    const root = getRoot();
    if (!frame || !root) return null;
    const f = frame.getBoundingClientRect();
    const r = root.getBoundingClientRect();
    return { x: f.left - r.left, y: f.top - r.top };
  }

  function insideFrame(p: Point, off: Point): boolean {
    const frame = getFrame();
    if (!frame) return false;
    const w = frame.clientWidth;
    const h = frame.clientHeight;
    return p.x >= off.x && p.y >= off.y && p.x <= off.x + w && p.y <= off.y + h;
  }

  return {
    async pick(a, b) {
      const off = frameOffset();
      if (!off || !insideFrame(a, off)) return null;
      const res = await ask({
        op: "pick",
        x: a.x - off.x,
        y: a.y - off.y,
        x2: b ? b.x - off.x : null,
        y2: b ? b.y - off.y : null,
      });
      return sanitizeFrameAnchor(res?.anchor);
    },

    async locate(comments) {
      const off = frameOffset();
      const ids = comments.map((c) => c.id);
      // 非表示（react の Code タブ中など）の iframe は座標が意味を持たない
      const hidden = !getFrame() || getFrame()!.clientWidth === 0;
      if (!off || hidden || comments.length === 0) return Object.fromEntries(ids.map((id) => [id, null]));
      const res = await ask({
        op: "locate",
        items: comments.map((c) => ({ id: c.id, anchor: c.anchor })),
      });
      const positions = sanitizeFramePositions(res?.positions, ids);
      const out: Record<string, FramePosition> = {};
      for (const id of ids) {
        const p = positions[id];
        out[id] = p ? { x: p.x + off.x, y: p.y + off.y, orphan: p.orphan } : null;
      }
      return out;
    },

    subscribe(cb) {
      const listener = (event: MessageEvent) => {
        onMessage(event);
        const frame = getFrame();
        const data = event.data as Record<string, unknown> | null;
        if (
          frame &&
          event.source === frame.contentWindow &&
          data &&
          typeof data === "object" &&
          data[ARTIFACT_COMMENT_PROBE_CHANGED_MARKER] === true
        ) {
          cb();
        }
      };
      window.addEventListener("message", listener);
      // iframe 自体の大きさの変化（親のリサイズ・CSP バナーの開閉）
      const root = getRoot();
      const ro = new ResizeObserver(() => cb());
      if (root) ro.observe(root);
      const frame = getFrame();
      if (frame) ro.observe(frame);
      return () => {
        window.removeEventListener("message", listener);
        ro.disconnect();
        for (const done of pending.values()) done({});
        pending.clear();
      };
    },
  };
}
