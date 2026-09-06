/**
 * HTML アーティファクト用の srcdoc を生成する。
 *
 * 生の content をそのまま srcdoc に流すと (1) CSP が一切かからず (2) `artifact:` リンクの
 * クリックを拾えない（sandbox に allow-top-navigation が無いので何も起きない）ため、
 * ドキュメントをパースして CSP メタとクリック横取りスクリプトを差し込む。
 *
 * 文字列連結ではなく DOMParser を使うのは、CSP メタが効くには `<head>` の先頭に置く必要が
 * あり、かつ `<!DOCTYPE html>` より前に何かを足すと quirks モードに落ちるため。
 */

import { ARTIFACT_LINK_INTERCEPT_JS } from "./artifactFrameLink";

/**
 * React アーティファクト（utils/reactArtifactSrcdoc.ts）と同じ CSP。
 * connect-src を含む default-src 'none' で外部通信を止め、sandbox="allow-scripts"
 * （allow-same-origin なし）と組み合わせて親ウィンドウから隔離する。
 */
export const HTML_ARTIFACT_CSP =
  "default-src 'none'; script-src 'unsafe-inline' 'unsafe-eval'; style-src 'unsafe-inline'; img-src data: blob:;";

export function buildHtmlSrcdoc(content: string): string {
  const doc = new DOMParser().parseFromString(content, "text/html");
  // DOMParser は content が断片でも必ず head / body を作る
  const head = doc.head;
  const body = doc.body;

  const meta = doc.createElement("meta");
  meta.setAttribute("http-equiv", "Content-Security-Policy");
  meta.setAttribute("content", HTML_ARTIFACT_CSP);
  head.insertBefore(meta, head.firstChild);

  const script = doc.createElement("script");
  script.textContent = ARTIFACT_LINK_INTERCEPT_JS;
  body.appendChild(script);

  return `<!DOCTYPE html>\n${doc.documentElement.outerHTML}`;
}
