/**
 * アーティファクトのコメント。実体は状態サイドカー `<id>.state` の `comments`
 * （Rust 側は `src-tauri/src/artifact_comments.rs`）。AI が本体を作り直しても消えない。
 */

export type CommentAuthor = "user" | "ai";

/**
 * コメントの位置。保存形は Rust 側が「8KB までの JSON オブジェクト」としか見ないので、
 * 形はここで決める。AI が付けるコメントは `excerpt` だけの場合もある（位置なし）。
 *
 * - `selector`: 描画ルートからの CSS パス。要素が見つからなくなったら `x` / `y` へ退避する
 * - `excerpt`: 付けた時点の対象のテキスト抜粋。AI が本体を書き換えた後でも何を指していたか追える
 */
export interface CommentAnchor {
  kind?: "point" | "element" | "region";
  /** 描画ルート内の位置（割合 0..1）。selector が解決できないときの退避先 */
  x?: number;
  y?: number;
  w?: number;
  h?: number;
  selector?: string;
  /** selector の要素内での位置（割合 0..1）。point のときだけ使う */
  rx?: number;
  ry?: number;
  excerpt?: string;
  /** アプリのコメントで、付けたウィンドウ（"main" か、サブウィンドウの ID） */
  window?: string;
}

export interface CommentReply {
  id: string;
  author: CommentAuthor;
  body: string;
  createdAt: number;
}

export interface ArtifactComment {
  id: string;
  author: CommentAuthor;
  body: string;
  anchor: CommentAnchor;
  createdAt: number;
  resolved: boolean;
  replies: CommentReply[];
}

/** `artifact-comments-changed` イベント（メモリーの `artifact-state-changed` とは別系統） */
export interface ArtifactCommentsChangedEvent {
  scope: "worktree" | "repository";
  scopeId: string;
  artifactId: string;
}
