/**
 * html アーティファクト用の CSP と、その違反を親ウィンドウへ伝えるブリッジ。
 *
 * srcdoc iframe は親の CSP を継承する（そして親は `"csp": null`）ため、CSP は
 * `<head>` 先頭の meta で自前に張るしかない。違反はコンソールにしか出ず画面には
 * 何も現れないので、`securitypolicyviolation` を拾って親へ渡し、バナーで見せる。
 *
 * リンクブリッジ（`artifactFrameLink.ts`）と同じく iframe は
 * `sandbox="allow-scripts"`（allow-same-origin なし）で origin は不透明な `"null"`。
 * 親側は必ず `event.source === iframe.contentWindow` で送信元を判定すること。
 *
 * ただし contentWindow の一致だけでは足りない。CSP を張る相手＝コンテンツは信頼できず、
 * 収集スクリプトを経由せず `parent.postMessage` を直接叩けるし、srcdoc を差し替えても
 * contentWindow は同一オブジェクトのままなので、前の文書が仕掛けた遅延メッセージが
 * 差し替え後に届く。そのため srcdoc ごとの nonce を突き合わせ、件数と文字列長は
 * 受信側でも必ず切り詰める。
 */

/**
 * html アーティファクトに張る CSP。
 *
 * react 側（`reactArtifactSrcdoc.ts`）と同一だが、`img-src` にだけ `https:` を足している。
 * 生 HTML は外部画像を貼るのが自然な一方、画像は「URL に載せて外へ送る」以上のことが
 * できず、iframe は opaque origin なので送れる中身もアーティファクト本文に限られる。
 * 逆に外部 CSS / Web フォントは `@import` と `url()` で攻撃面が広がるだけなので許さない。
 * `connect-src 'none'` は送信の遮断というより、外部の応答を読んで挙動を変える経路を
 * 作らせないために残している。
 */
export const HTML_ARTIFACT_CSP =
  "default-src 'none'; " +
  "script-src 'unsafe-inline' 'unsafe-eval'; " +
  "style-src 'unsafe-inline'; " +
  "img-src data: blob: https:; " +
  "connect-src 'none';";

/** postMessage のペイロードを他のメッセージと区別するためのマーカー */
export const ARTIFACT_CSP_VIOLATION_MARKER = "__oretachiArtifactCspViolation";

export interface ArtifactCspViolation {
  /** 違反したディレクティブ（例: `script-src`） */
  directive: string;
  /** ブロックされた URL。インライン違反などでは空文字や `inline` になる */
  blockedUri: string;
}

export interface ArtifactCspViolationReport {
  violations: ArtifactCspViolation[];
  /** 上限に達して報告を打ち切ったか（バナーで「N 種類以上」と出すため） */
  truncated: boolean;
}

export interface ArtifactCspViolationMessage extends ArtifactCspViolationReport {
  [ARTIFACT_CSP_VIOLATION_MARKER]: true;
  nonce: string;
}

/** 1つのアーティファクトから受け取る違反の上限（暴走した文書で親を溢れさせない） */
export const ARTIFACT_CSP_VIOLATION_LIMIT = 50;

/** ディレクティブ名と URL の保持上限（超長文字列をバナーへ流し込ませない） */
export const ARTIFACT_CSP_TEXT_LIMIT = 200;

/** srcdoc ごとに使い捨てる識別子。前の文書からの遅延メッセージを弾くために使う */
export function createCspNonce(): string {
  const c = globalThis.crypto;
  if (c && typeof c.randomUUID === "function") return c.randomUUID();
  return `${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
}

function clipText(value: unknown): string {
  const s = typeof value === "string" ? value : "";
  return s.length > ARTIFACT_CSP_TEXT_LIMIT ? s.slice(0, ARTIFACT_CSP_TEXT_LIMIT) : s;
}

/**
 * iframe 内に注入する違反収集スクリプトを作る。
 * meta CSP の直後、他のどのリソースよりも前に置くこと（後続の違反しか拾えないため）。
 *
 * 実際に呼ぶのは 50ms 後なので、コンテンツに差し替えられる前に `setTimeout` と
 * `parent.postMessage` を束縛しておく。
 */
export function buildCspViolationReportJs(nonce: string): string {
  return (
    "(function(){" +
    "  var MAX=" + ARTIFACT_CSP_VIOLATION_LIMIT + ";" +
    "  var TEXT=" + ARTIFACT_CSP_TEXT_LIMIT + ";" +
    "  var NONCE=" + JSON.stringify(nonce) + ";" +
    "  var MARKER=" + JSON.stringify(ARTIFACT_CSP_VIOLATION_MARKER) + ";" +
    "  var st=setTimeout;var target=parent;var has=Object.prototype.hasOwnProperty;" +
    "  function send(m){try{target.postMessage(m,'*');}catch(err){}}" +
    "  var pending=[];var keys={};var count=0;var truncated=false;var timer=null;" +
    "  function flush(){" +
    "    timer=null;" +
    "    var batch=pending;pending=[];" +
    "    var msg={nonce:NONCE,violations:batch,truncated:truncated};" +
    "    msg[MARKER]=true;" +
    "    send(msg);" +
    "  }" +
    "  function schedule(){if(timer===null)timer=st(flush,50);}" +
    "  function clip(v){var s=String(v==null?'':v);return s.length>TEXT?s.slice(0,TEXT):s;}" +
    "  document.addEventListener('securitypolicyviolation',function(e){" +
    "    var d=clip(e.effectiveDirective||e.violatedDirective||'');" +
    "    var u=clip(e.blockedURI||'');" +
    // 同じディレクティブ・同じ URL は親側でも1件に畳まれる。
    // 枠を数え上げるのは畳んだ後の件数でないと、1つの URL の連打で枠を使い切ってしまう
    "    var key=d+'|'+u;" +
    "    if(has.call(keys,key))return;" +
    "    if(count>=MAX){if(!truncated){truncated=true;schedule();}return;}" +
    "    keys[key]=true;count++;" +
    "    pending.push({directive:d,blockedUri:u});" +
    "    schedule();" +
    "  });" +
    "})();"
  );
}

/**
 * `message` イベントが対象 iframe の現在の文書から来た CSP 違反通知なら、その内容を返す。
 * そうでなければ null。
 */
export function readArtifactCspViolationMessage(
  event: MessageEvent,
  frame: HTMLIFrameElement | null,
  nonce: string,
): ArtifactCspViolationReport | null {
  if (!frame || event.source !== frame.contentWindow) return null;
  const data = event.data as Partial<ArtifactCspViolationMessage> | null;
  if (!data || typeof data !== "object" || data[ARTIFACT_CSP_VIOLATION_MARKER] !== true) return null;
  // 前の srcdoc が差し替え直前に仕掛けた setTimeout はここで落ちる
  if (typeof data.nonce !== "string" || data.nonce !== nonce) return null;
  if (!Array.isArray(data.violations)) return null;

  // 収集スクリプトを介さず直接 postMessage された巨大配列で親を止めないよう、
  // 走査そのものを上限で打ち切る
  const raws = data.violations.slice(0, ARTIFACT_CSP_VIOLATION_LIMIT);
  const violations: ArtifactCspViolation[] = [];
  for (const raw of raws) {
    if (!raw || typeof raw !== "object") continue;
    const { directive, blockedUri } = raw as ArtifactCspViolation;
    if (typeof directive !== "string" || typeof blockedUri !== "string") continue;
    violations.push({ directive: clipText(directive), blockedUri: clipText(blockedUri) });
  }

  const truncated = data.truncated === true || data.violations.length > ARTIFACT_CSP_VIOLATION_LIMIT;
  if (violations.length === 0 && !truncated) return null;
  return { violations, truncated };
}

/** 同じディレクティブ・同じ URL の違反をまとめ、発生順を保ったまま件数を数える */
export function mergeCspViolations(
  current: ArtifactCspViolation[],
  incoming: ArtifactCspViolation[],
): ArtifactCspViolation[] {
  const merged = [...current];
  const keys = new Set(merged.map((v) => v.directive + "|" + v.blockedUri));
  for (const v of incoming) {
    const key = v.directive + "|" + v.blockedUri;
    if (keys.has(key)) continue;
    if (merged.length >= ARTIFACT_CSP_VIOLATION_LIMIT) break;
    keys.add(key);
    merged.push(v);
  }
  return merged;
}
