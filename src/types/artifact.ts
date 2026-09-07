/** URL アーティファクトの content_type（Rust 側は artifact_url.rs の URL_ARTIFACT_CONTENT_TYPE） */
export const URL_ARTIFACT_CONTENT_TYPE = "text/uri-list";

/** アイコンボタンのドロップダウンに並べる URL アーティファクト */
export interface UrlArtifactEntry {
  id: string;
  title: string;
  url: string;
}

export interface ArtifactMeta {
  id: string;
  title: string;
  content_type: string;
  language?: string;
  created_at: number;
  updated_at: number;
  /** リポジトリへ転送されたアーティファクトのみ持つ、転送元ワークツリーの ID */
  source_worktree_id?: string;
  /**
   * 表示中ロックの永続フラグ。true のアーティファクトは、ビューアで開かれている間
   * MCP 由来の書き込み（update / rewrite / artifact_module / artifact_store）を拒否する。
   * 判定は Rust 側（`artifact_lock::ArtifactOpenRegistry` との AND）で行う。
   */
  locked_while_open?: boolean;
}

export interface ArtifactData extends ArtifactMeta {
  content: string;
  modules?: Record<string, string>;
}

/**
 * アーティファクト本体（AI 所有の `<id>.json`）とは別ファイルに置く、UI / 人が所有する可変状態。
 * 実体は `<id>.state`（Rust 側 `src-tauri/src/lib.rs` の状態サイドカー）。
 * バージョン番号は持たず、未知のキーは読み飛ばし・欠けたキーは既定値で補う。
 */
export interface ArtifactState {
  pinned?: boolean;
  /**
   * React アーティファクトのメモリー（フォーム入力などの復元用 JSON ストア）。
   * ピン止めと違い転送でも引き継ぐ（アーティファクトの中身に属する状態のため）。
   */
  memory?: Record<string, unknown>;
  /**
   * `memory` の最終更新時刻（epoch ミリ秒）。MCP の `artifact_store` が
   * `expected_updated_at`（楽観ロック）で突き合わせる値。
   */
  memoryUpdatedAt?: number;
}

/** MCP の `artifact_store` がストアを書き換えたときに飛ぶイベント */
export interface ArtifactStateChangedEvent {
  scope: "worktree" | "repository";
  scopeId: string;
  artifactId: string;
}

export interface ArtifactChangedEvent {
  worktreeId: string;
  artifactId: string;
  command: string;
}

export interface RepoArtifactChangedEvent {
  repositoryId: string;
  artifactId: string;
  command: string;
}

/** copy_artifact_to_repository の戻り値 */
export interface CopyArtifactResult {
  status: "copied" | "exists";
  repositoryId: string;
  repositoryName: string;
}
