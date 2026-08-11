import { URL_ARTIFACT_CONTENT_TYPE, type UrlArtifactEntry } from "../types/artifact";

/**
 * `list_artifacts` / `list_repo_artifacts` の生 JSON 配列から URL アーティファクトだけを取り出す。
 * `list_artifacts` は content 込みで返す。`list_repo_artifacts` は通常 content を落とすが、
 * `text/uri-list` だけは例外的に残す（src-tauri/src/lib.rs の list_repo_artifacts）ので、
 * どちらも追加の read なしで URL を取れる。
 * JSON 側のキーは `type`（Rust の serde rename）である点に注意。
 */
export function extractUrlArtifacts(list: unknown[]): UrlArtifactEntry[] {
  const entries: UrlArtifactEntry[] = [];
  for (const raw of list) {
    if (!raw || typeof raw !== "object") continue;
    const obj = raw as Record<string, unknown>;
    const contentType = obj.type ?? obj.content_type;
    if (contentType !== URL_ARTIFACT_CONTENT_TYPE) continue;

    const id = typeof obj.id === "string" ? obj.id : "";
    const content = typeof obj.content === "string" ? obj.content : "";
    const url = content.trim().split(/\r?\n/)[0]?.trim() ?? "";
    // アーティファクト本文は AI が自由に書けるため、ブラウザで開く前にスキームを絞る
    if (!id || !/^https?:\/\/\S+$/.test(url)) continue;

    const title = typeof obj.title === "string" && obj.title.trim() ? obj.title : url;
    entries.push({ id, title, url });
  }
  return entries;
}
