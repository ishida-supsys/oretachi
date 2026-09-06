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

/** `artifact:` スキームかどうかの判定だけが要る呼び出し元向け（解析の成否は問わない） */
export const ARTIFACT_SCHEME_RE = /^artifact:/i;

export type ArtifactScope = "worktree" | "repository";

export interface ArtifactLinkTarget {
  /** 短縮形（同一スコープ内）なら null */
  scope: ArtifactScope | null;
  /** 短縮形なら null。worktree スコープなら worktreeId、repository スコープなら repositoryId */
  id: string | null;
  artifactId: string;
}

/**
 * アーティファクト ID はファイル名にそのまま使われるため、パス区切り・`..`・
 * NTFS の代替データストリーム（`:`）・制御文字を弾く。
 *
 * Rust 側の `validate_path_component` にも同じ検証があるが、こちらは
 * 「このパーサが返した ID は安全」という前提を単体で成立させておくためのもの。
 */
// スラッシュ / コロン / バックスラッシュ。エスケープの取り違えを避けるため
// 正規表現ではなくコードポイントで持つ
const FORBIDDEN_ID_CODES = new Set([0x2f, 0x3a, 0x5c]);

function isValidArtifactId(id: string): boolean {
  if (id === "" || id === "." || id === "..") return false;
  for (const ch of id) {
    const code = ch.codePointAt(0) ?? 0;
    // 制御文字（NUL・改行など）もファイル名には載せない
    if (code < 0x20 || code === 0x7f || FORBIDDEN_ID_CODES.has(code)) return false;
  }
  return true;
}

/** ワークツリー ID も同じ制約で扱う（リポジトリ ID は絶対パスなので対象外） */
function isValidWorktreeId(id: string): boolean {
  return isValidArtifactId(id);
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
  if (!ARTIFACT_SCHEME_RE.test(trimmed)) return null;

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
  if (!isValidArtifactId(artifactId)) return null;
  // ワークツリー ID はウィンドウラベルにも使うので ID 制約を掛ける。
  // リポジトリ ID は絶対パスなので空でないことだけ見る
  if (rawScope === "worktree" ? !isValidWorktreeId(id) : id === "") return null;

  return { scope: rawScope, id, artifactId };
}
