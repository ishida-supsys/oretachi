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
 * srcdoc iframe は親の CSP を継承するが親は `"csp": null` なので、CSP は
 * `<head>` 先頭の meta で自前に張る（`HTML_ARTIFACT_CSP`）。違反は画面に何も出ず
 * 「急に崩れた」としか見えないため、収集スクリプトも合わせて注入し、親側で
 * バナーに出す。srcdoc を差し替えても iframe の contentWindow は同一のままで、
 * 前の文書が仕掛けた遅延メッセージが差し替え後に届きうるため、呼び出しごとに
 * nonce を発行して返し、親側で突き合わせられるようにする。
 */

import { ARTIFACT_LINK_INTERCEPT_JS } from "./artifactFrameLink";
import {
  buildCspViolationReportJs,
  createCspNonce,
  HTML_ARTIFACT_CSP,
} from "./artifactCspViolation";

export interface HtmlArtifactSrcdoc {
  srcdoc: string;
  /** この srcdoc の違反通知だけを受け入れるための使い捨て識別子 */
  nonce: string;
}

export function buildHtmlSrcdoc(content: string): HtmlArtifactSrcdoc {
  const doc = new DOMParser().parseFromString(content, "text/html");

  const script = doc.createElement("script");
  script.textContent = ARTIFACT_LINK_INTERCEPT_JS;
  // <frameset> 文書では body の代わりに frameset が入っており、その子の <script> は
  // 再パース時に捨てられる。その場合だけ head の末尾へ逃がす
  const host =
    doc.body && doc.body.tagName !== "FRAMESET" ? doc.body : doc.head;
  host.appendChild(script);

  // 違反収集は meta より後・他のどのリソースより前でなければ取りこぼす。
  // meta → 収集スクリプトの順になるよう、逆順に head の先頭へ差し込む
  const nonce = createCspNonce();
  const reporter = doc.createElement("script");
  reporter.textContent = buildCspViolationReportJs(nonce);
  doc.head.insertBefore(reporter, doc.head.firstChild);

  const meta = doc.createElement("meta");
  meta.setAttribute("http-equiv", "Content-Security-Policy");
  meta.setAttribute("content", HTML_ARTIFACT_CSP);
  doc.head.insertBefore(meta, doc.head.firstChild);

  // 元の doctype は尊重する（無い断片だけ標準モードになるよう html を補う）
  const doctype = doc.doctype
    ? `<!DOCTYPE ${doc.doctype.name}${doc.doctype.publicId ? ` PUBLIC "${doc.doctype.publicId}"` : ""}${doc.doctype.systemId ? ` "${doc.doctype.systemId}"` : ""}>`
    : "<!DOCTYPE html>";

  return { srcdoc: `${doctype}\n${doc.documentElement.outerHTML}`, nonce };
}
