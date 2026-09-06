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
