/**
 * アーティファクト本文に書ける `artifact:` リンクの解析。
 *
 * 形式:
 *   完全形  artifact://worktree/<worktreeId>/<artifactId>
 *           artifact://repository/<encodeURIComponent(repositoryId)>/<artifactId>
 *   短縮形  artifact:<artifactId>            ← 同一スコープ内
 *
 * 完全形は必ず `//` で始まり、短縮形は `//` を持たないので、この1点だけで判別できる。
 * repositoryId は絶対パス（`X:\devel\...`）なので、URL に載せる際は必ず
 * encodeURIComponent する（`/` や `\` がパス区切りと衝突するため）。
 *
 * 判定は必ず `getAttribute("href")` の生値に対して行うこと。要素の `.href` プロパティは
 * ブラウザが解決済みの値を返し、未知スキームの扱いが環境依存になる
 * （utils/externalLink.ts と同じ方針）。
 */

export type ArtifactScope = "worktree" | "repository";

export interface ArtifactLinkTarget {
  /** 短縮形（同一スコープ内）なら null */
  scope: ArtifactScope | null;
  /** 短縮形なら null。worktree スコープなら worktreeId、repository スコープなら repositoryId */
  id: string | null;
  artifactId: string;
}

/** アーティファクト ID はファイル名にそのまま使われるため、パス区切りと `..` を弾く */
function isValidArtifactId(id: string): boolean {
  if (id === "" || id === "." || id === "..") return false;
  return !/[\\/]/.test(id);
}

function safeDecode(value: string): string | null {
  try {
    return decodeURIComponent(value);
  } catch {
    // `%` 単体などで decodeURIComponent が投げる。リンクとしては無効扱いにする
    return null;
  }
}

/**
 * `<a href>` の生値を `artifact:` リンクとして解析する。リンクでない・壊れているなら null。
 */
export function parseArtifactLink(href: string | null | undefined): ArtifactLinkTarget | null {
  if (typeof href !== "string") return null;
  const trimmed = href.trim();
  if (!/^artifact:/i.test(trimmed)) return null;

  const rest = trimmed.slice("artifact:".length);

  // ── 短縮形: artifact:<artifactId> ──
  if (!rest.startsWith("//")) {
    const artifactId = safeDecode(rest);
    if (artifactId === null || !isValidArtifactId(artifactId)) return null;
    return { scope: null, id: null, artifactId };
  }

  // ── 完全形: artifact://<scope>/<id>/<artifactId> ──
  const parts = rest.slice(2).split("/");
  if (parts.length !== 3) return null;
  const [rawScope, rawId, rawArtifactId] = parts;
  if (rawScope !== "worktree" && rawScope !== "repository") return null;

  const id = safeDecode(rawId);
  const artifactId = safeDecode(rawArtifactId);
  if (id === null || artifactId === null) return null;
  if (id === "" || !isValidArtifactId(artifactId)) return null;

  return { scope: rawScope, id, artifactId };
}

/** 解析結果を人間が読める形に戻す（トーストのメッセージ用） */
export function formatArtifactLink(target: ArtifactLinkTarget): string {
  if (target.scope === null) return `artifact:${target.artifactId}`;
  return `artifact://${target.scope}/${encodeURIComponent(target.id ?? "")}/${target.artifactId}`;
}
