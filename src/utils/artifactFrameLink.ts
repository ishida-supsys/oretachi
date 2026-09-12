/**
 * sandbox iframe（html / react アーティファクト）内の `artifact:` リンククリックを
 * 親ウィンドウへ伝えるためのブリッジ。
 *
 * iframe は `sandbox="allow-scripts"`（allow-same-origin なし）なので origin は不透明な
 * `"null"` になり、`event.origin` による検証は使えない。親側は必ず
 * `event.source === iframe.contentWindow` で送信元を判定すること。
 *
 * 文字列で持つのは、この JS を srcdoc に直接埋め込むため。Vue SFC パーサーとの干渉を
 * 避ける目的で `utils/reactArtifactSrcdoc.ts` と同じく SFC の外に置いている。
 */

/** postMessage のペイロードを他のメッセージと区別するためのマーカー */
export const ARTIFACT_NAVIGATE_MARKER = "__oretachiArtifactNavigate";

export interface ArtifactFrameNavigateMessage {
  [ARTIFACT_NAVIGATE_MARKER]: true;
  href: string;
}

/** リンクのホバー通知（URL 表示 + コピーのポップアップ用）のマーカー */
export const ARTIFACT_LINK_HOVER_MARKER = "__oretachiArtifactLinkHover";

/** iframe 内のリンクの位置。iframe 自身のビューポート座標なので、親側で iframe の分だけずらす */
export interface ArtifactLinkRect {
  left: number;
  top: number;
  width: number;
  height: number;
}

export interface ArtifactFrameLinkHoverMessage {
  [ARTIFACT_LINK_HOVER_MARKER]: true;
  /** null なら「リンクから離れた」= ポップアップを閉じる要求 */
  href: string | null;
  rect: ArtifactLinkRect | null;
}

/** ホバーの結果。href が null なら閉じる要求 */
export interface ArtifactLinkHover {
  href: string | null;
  rect: ArtifactLinkRect | null;
}

/**
 * iframe 内に注入するリンク横取りスクリプト。
 *
 * クリックはページ内アンカー（`#...`）を除く全リンクで不発にする。sandbox は外部への
 * 「遷移」を塞ぐが、塞いだ結果 iframe の表示が壊れる（about:blank 相当に化ける）ため、
 * こちらで先に止める必要がある。`artifact:` だけは親へ転送して遷移を代行してもらう。
 * 外部 URL を開きたい場合は、ホバー通知で親が出すポップアップ側の URL を押す。
 *
 * ホバー通知はどのリンクでも親へ送る（親が URL・コピー・ブラウザで開くを出す）。
 */
export const ARTIFACT_LINK_INTERCEPT_JS =
  "(function(){" +
  "  function findAnchor(e){" +
  "    var path=(typeof e.composedPath==='function')?e.composedPath():null;" +
  "    if(path){" +
  "      for(var i=0;i<path.length;i++){" +
  "        var n=path[i];" +
  "        if(n&&n.nodeType===1&&n.tagName==='A'&&n.hasAttribute('href'))return n;" +
  "      }" +
  "      return null;" +
  "    }" +
  "    var el=e.target;" +
  "    return (el&&el.closest)?el.closest('a[href]'):null;" +
  "  }" +
  "  function onClick(e){" +
  "    var a=findAnchor(e);" +
  "    if(!a)return;" +
  "    var href=(a.getAttribute('href')||'').trim();" +
  // ページ内アンカーだけは素通しする（同じ文書内のジャンプで表示は壊れない）
  "    if(href.charAt(0)==='#')return;" +
  "    e.preventDefault();" +
  "    e.stopPropagation();" +
  "    if(!/^artifact:/i.test(href))return;" +
  "    try{parent.postMessage({" + JSON.stringify(ARTIFACT_NAVIGATE_MARKER) + ":true,href:href},'*');}catch(err){}" +
  "  }" +
  // 中クリックは click ではなく auxclick で飛ぶ（ArtifactMarkdownView と同じ理由）
  "  document.addEventListener('click',onClick,true);" +
  "  document.addEventListener('auxclick',onClick,true);" +
  // ここから下はリンクのホバー通知。sandbox 内では URL を確かめる手段が
  // ステータスバーもタイトル属性も無く（外部遷移も塞がれている）、親側の
  // ポップアップに URL とコピーボタンを出してもらうしかない
  "  var hoveredHref=null;" +
  "  function post(msg){try{parent.postMessage(msg,'*');}catch(err){}}" +
  "  function hideHover(){" +
  "    if(hoveredHref===null)return;" +
  "    hoveredHref=null;" +
  "    post({" + JSON.stringify(ARTIFACT_LINK_HOVER_MARKER) + ":true,href:null,rect:null});" +
  "  }" +
  "  function onOver(e){" +
  "    var a=findAnchor(e);" +
  "    if(!a){hideHover();return;}" +
  "    var href=(a.getAttribute('href')||'').trim();" +
  // ページ内アンカーは飛び先が同じ文書なので出さない（親側の markdown ビューと同じ扱い）
  "    if(!href||href.charAt(0)==='#'){hideHover();return;}" +
  "    var r=a.getBoundingClientRect();" +
  "    hoveredHref=href;" +
  "    post({" + JSON.stringify(ARTIFACT_LINK_HOVER_MARKER) + ":true,href:href," +
  "      rect:{left:r.left,top:r.top,width:r.width,height:r.height}});" +
  "  }" +
  "  function onOut(e){if(findAnchor(e))hideHover();}" +
  "  document.addEventListener('mouseover',onOver,true);" +
  "  document.addEventListener('mouseout',onOut,true);" +
  // 位置が動いた/フォーカスが外れたら座標が合わなくなるので閉じる
  "  document.addEventListener('scroll',hideHover,true);" +
  "  window.addEventListener('blur',hideHover);" +
  "})();";

/**
 * `message` イベントが対象 iframe から来た遷移要求なら href を返す。そうでなければ null。
 */
export function readArtifactNavigateMessage(
  event: MessageEvent,
  frame: HTMLIFrameElement | null,
): string | null {
  if (!frame || event.source !== frame.contentWindow) return null;
  const data = event.data as Partial<ArtifactFrameNavigateMessage> | null;
  if (!data || typeof data !== "object" || data[ARTIFACT_NAVIGATE_MARKER] !== true) return null;
  return typeof data.href === "string" ? data.href : null;
}

/**
 * `message` イベントが対象 iframe から来たリンクのホバー通知なら中身を返す。そうでなければ null。
 * 座標は iframe 自身のビューポート基準なので、親側で iframe の位置を足すこと。
 */
export function readArtifactLinkHoverMessage(
  event: MessageEvent,
  frame: HTMLIFrameElement | null,
): ArtifactLinkHover | null {
  if (!frame || event.source !== frame.contentWindow) return null;
  const data = event.data as Partial<ArtifactFrameLinkHoverMessage> | null;
  if (!data || typeof data !== "object" || data[ARTIFACT_LINK_HOVER_MARKER] !== true) return null;
  if (data.href === null || data.href === undefined) return { href: null, rect: null };
  if (typeof data.href !== "string") return null;
  const rect = readRect(data.rect);
  // 座標が壊れている通知は「閉じる」として扱う（変な位置に出すより閉じる方が無害）
  return rect ? { href: data.href, rect } : { href: null, rect: null };
}

function readRect(rect: unknown): ArtifactLinkRect | null {
  if (!rect || typeof rect !== "object") return null;
  const r = rect as Record<string, unknown>;
  const values = [r.left, r.top, r.width, r.height];
  if (!values.every((v) => typeof v === "number" && Number.isFinite(v))) return null;
  return {
    left: r.left as number,
    top: r.top as number,
    width: r.width as number,
    height: r.height as number,
  };
}
