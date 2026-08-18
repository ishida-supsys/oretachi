/**
 * <a href> の生の属性値を見て、外部ブラウザで開くべき URL を返す。開かないなら null。
 *
 * 判定は必ず getAttribute("href") の生値に対して行うこと。要素の .href プロパティは
 * ブラウザが document.baseURI で解決してしまい、相対パス "./a.md" が
 * "http://localhost:1420/a.md" に化けて外部リンクと誤判定される。
 *
 * アーティファクト本文は AI が自由に書けるため、javascript: などが特権ドキュメントから
 * 実行されないようスキームを http(s) に絞る (utils/artifactUrl.ts の
 * extractUrlArtifacts と同じ方針)。
 */
export function resolveExternalLink(href: string | null | undefined): string | null {
  if (typeof href !== "string") return null;
  const url = href.trim();
  return /^https?:\/\/\S+$/i.test(url) ? url : null;
}
