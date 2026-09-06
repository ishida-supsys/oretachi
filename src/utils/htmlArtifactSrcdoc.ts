/**
 * HTML アーティファクト用の srcdoc を生成する。
 *
 * 生の content をそのまま srcdoc に流すと `artifact:` リンクのクリックを拾えない
 * （sandbox に allow-top-navigation が無いので何も起きない）ため、ドキュメントを
 * パースしてクリック横取りスクリプトを差し込む。
 *
 * 文字列連結ではなく DOMParser を使うのは、`<!DOCTYPE html>` より前に何かを足すと
 * quirks モードに落ちるため。パース結果の文書は browsing context を持たないので、
 * ここでスクリプトが走ったり外部リソースを取りに行ったりはしない。
 *
 * なお html view には CSP が無い（親も `"csp": null`）。React 側と揃える案はあるが、
 * 既存の HTML アーティファクトは CDN 前提で書かれているものが多く、無言で崩れる。
 * CSP の付与は影響範囲を調べたうえで別途対応する。
 */

import { ARTIFACT_LINK_INTERCEPT_JS } from "./artifactFrameLink";

export function buildHtmlSrcdoc(content: string): string {
  const doc = new DOMParser().parseFromString(content, "text/html");

  const script = doc.createElement("script");
  script.textContent = ARTIFACT_LINK_INTERCEPT_JS;
  // <frameset> 文書では body の代わりに frameset が入っており、その子の <script> は
  // 再パース時に捨てられる。その場合だけ head の末尾へ逃がす
  const host =
    doc.body && doc.body.tagName !== "FRAMESET" ? doc.body : doc.head;
  host.appendChild(script);

  // 元の doctype は尊重する（無い断片だけ標準モードになるよう html を補う）
  const doctype = doc.doctype
    ? `<!DOCTYPE ${doc.doctype.name}${doc.doctype.publicId ? ` PUBLIC "${doc.doctype.publicId}"` : ""}${doc.doctype.systemId ? ` "${doc.doctype.systemId}"` : ""}>`
    : "<!DOCTYPE html>";

  return `${doctype}\n${doc.documentElement.outerHTML}`;
}
