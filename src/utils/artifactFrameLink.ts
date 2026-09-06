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

/**
 * iframe 内に注入するクリック横取りスクリプト。
 * `artifact:` 以外のリンクには触れない（sandbox が外部遷移を既に塞いでいる）。
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
  "    if(!/^artifact:/i.test(href))return;" +
  "    e.preventDefault();" +
  "    e.stopPropagation();" +
  "    try{parent.postMessage({" + JSON.stringify(ARTIFACT_NAVIGATE_MARKER) + ":true,href:href},'*');}catch(err){}" +
  "  }" +
  // 中クリックは click ではなく auxclick で飛ぶ（ArtifactMarkdownView と同じ理由）
  "  document.addEventListener('click',onClick,true);" +
  "  document.addEventListener('auxclick',onClick,true);" +
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
