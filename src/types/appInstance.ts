/**
 * アプリ形式アーティファクト（manifest の `app`）の型。
 * Rust 側は `src-tauri/src/app_manifest.rs` / `app_instance.rs`（camelCase で直列化）。
 *
 * このファイルは Tauri にも Web にも依存しない（Web ビューアのバンドルからも import される）。
 */

/** 承認時に解決した越境先 1 件 */
export interface AppResolvedTarget {
  /** manifest に書かれていた生文字列 */
  raw: string;
  /** 照合に使う正規化済みの形（ワークツリー ID / `*` / `workgroup:<gid>` / `repo:<名前>`） */
  stored: string;
  /** 表示名（承認時点のワークツリー名など） */
  label: string;
  /** `*` / `workgroup:` / `repo:`。承認後に作られるワークツリーにも当たる */
  wildcard: boolean;
}

/** 承認時に解決した FS のスコープ 1 件。`path` は解決済みの絶対パス（Windows では拡張長パス接頭辞付きのことがある） */
export interface AppResolvedFsScope {
  scheme: "worktree" | "artifact";
  /** 正規化済みの相対プレフィックス（`out/`。ルート全体は空） */
  prefix: string;
  path: string;
  write: boolean;
}

export interface AppResolvedFs {
  worktreeRoot?: string;
  artifactRoot?: string;
  scopes: AppResolvedFsScope[];
  /** 既定拒否パス（`.git/` など）への書き込みを許すか。最も強い警告の対象 */
  allowSensitivePaths: boolean;
}

export interface AppResolvedPermissions {
  mcp?: { tools: string[]; targets: AppResolvedTarget[] };
  memory?: { maxBytes: number };
  fs?: AppResolvedFs;
  /** DB ファイルの置き場所のスコープ（形は fs と同じ。read のスコープの DB は読み取り専用） */
  db?: AppResolvedFs;
}

export interface AppRequestedPermissions {
  mcp?: { tools: string[]; targets: string[] };
  memory?: { maxBytes: number };
  fs?: { read: string[]; write: string[]; allowSensitivePaths: boolean };
  db?: { read: string[]; write: string[]; allowSensitivePaths: boolean };
}

export interface AppGrant {
  /** UNIX 秒 */
  grantedAt: number;
  requested: AppRequestedPermissions;
  resolved: AppResolvedPermissions;
}

export interface AppInstanceStatus {
  worktreeId: string;
  artifactId: string;
  running: boolean;
  state: "starting" | "running" | null;
  /** 実行ウィンドウが表示されているか */
  visible: boolean;
  /** manifest の権限が変わり、再承認を待っているか（保留中は旧セットで動作する） */
  reapprovalPending: boolean;
  granted: boolean;
  grantedAt: number | null;
  maxInstances: number;
  /** コード更新・再承認でリロードした回数と最後の時刻（UNIX ミリ秒） */
  reloadCount: number;
  lastReloadAt: number | null;
  /** 承認済みのメモリー最大サイズ（memory 権限が無ければ null） */
  memoryMaxBytes: number | null;
  /** 直近に測ったメモリーの使用量（未測定は null） */
  memoryBytes: number | null;
  /** 上限を下げた結果、既存データが上限を超えている。書き込みだけ拒否され、読み取りはできる */
  memoryOverCap: boolean;
}

export interface AppLogEntry {
  /** UNIX ミリ秒 */
  ts: number;
  level: "info" | "warn" | "error";
  message: string;
}

/** アーティファクト JSON の `app`。**AI が書いたものなので表示専用**（権限の根拠にはしない） */
export interface AppManifestView {
  name?: string;
  description?: string;
  version?: string;
  source?: { skill?: string; version?: string };
}

/** main に出す承認ダイアログの要求（Rust の `app-grant-request` イベント） */
export interface AppGrantRequest {
  requestId: string;
  /** "start": 起動時（初回・権限変更後） / "change": 起動中の権限変更 */
  kind: "start" | "change";
  worktreeId: string;
  worktreeName: string;
  artifactId: string;
  artifactTitle: string;
  appName: string;
  appVersion: string | null;
  appDescription: string | null;
  source: { skill?: string; version?: string } | null;
  requested: AppRequestedPermissions;
  resolved: AppResolvedPermissions;
  /** 再承認のとき、現在有効な権限 */
  previous: AppResolvedPermissions | null;
  /** 「ユーザーと同等の権限（任意コマンド実行）」として強調するツール */
  dangerousTools: string[];
}

/** 設定画面の承認一覧 */
export interface AppGrantListItem {
  worktreeId: string;
  worktreeName: string;
  artifactId: string;
  artifactTitle: string;
  grantedAt: number;
  resolved: AppResolvedPermissions;
  running: boolean;
}

/** 起動画面（ビューア）が使うアプリ操作。Tauri 実装だけが提供する（Web は起動不可） */
export interface AppLaunchApi {
  status(artifactId: string): Promise<AppInstanceStatus>;
  start(artifactId: string): Promise<void>;
  stop(artifactId: string): Promise<boolean>;
  toggleVisible(artifactId: string): Promise<boolean>;
  log(artifactId: string): Promise<AppLogEntry[]>;
  grant(artifactId: string): Promise<AppGrant | null>;
  revoke(artifactId: string): Promise<boolean>;
  onChanged(handler: (status: AppInstanceStatus) => void): Promise<() => void>;
  onLog(handler: (args: { artifactId: string; entry: AppLogEntry }) => void): Promise<() => void>;
}
