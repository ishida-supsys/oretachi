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

/** インスタンス専用 AI ターミナル。ユーザーと同じ権限で動くエージェントを起動できる */
export interface AppAiTerminalPermission {
  /** v1 は `["claude"]` のみ */
  agents: string[];
  /** 同時に開ける数 */
  maxConcurrent: number;
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

/** 専用サブウィンドウの申請（承認した値そのもの。解決で変わるものがない） */
export interface AppSubWindowPermission {
  maxCount: number;
  maxWidth: number;
  maxHeight: number;
  transparent: boolean;
  frameless: boolean;
  alwaysOnTop: boolean;
  position: boolean;
}

/** マイクの生音声の取得の申請（承認した値そのもの。解決で変わるものがない） */
export interface AppAudioPermission {
  microphone: true;
  /** 1 回の録音の最大秒数。host が強制する */
  maxSeconds: number;
  /** `audio.start` で指定できるサンプルレート（Hz） */
  sampleRates: number[];
}

/**
 * 動画・音声の再生（`<video>` / `<audio>` の src に直接使える）。fs / network の権限とは独立。
 * `fs` は `read` のスコープを解決した形（読み取り専用。無ければローカルファイルは不可）、
 * `allow` は http(s) の URL プレフィックス（正規化済み）。
 */
export interface AppResolvedMedia {
  fs?: AppResolvedFs;
  allow: string[];
}

export interface AppResolvedPermissions {
  mcp?: { tools: string[]; targets: AppResolvedTarget[] };
  memory?: { maxBytes: number };
  aiTerminal?: AppAiTerminalPermission;
  fs?: AppResolvedFs;
  /** DB ファイルの置き場所のスコープ（形は fs と同じ。read のスコープの DB は読み取り専用） */
  db?: AppResolvedFs;
  /** Rust プロキシが通す URL プレフィックス（正規化済み） */
  network?: { allow: string[] };
  subWindow?: AppSubWindowPermission;
  audio?: AppAudioPermission;
  media?: AppResolvedMedia;
}

export interface AppRequestedPermissions {
  mcp?: { tools: string[]; targets: string[] };
  memory?: { maxBytes: number };
  aiTerminal?: AppAiTerminalPermission;
  fs?: { read: string[]; write: string[]; allowSensitivePaths: boolean };
  db?: { read: string[]; write: string[]; allowSensitivePaths: boolean };
  network?: { allow: string[] };
  subWindow?: AppSubWindowPermission;
  audio?: AppAudioPermission;
  media?: { read: string[]; allow: string[] };
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
  /** マイクを録音中か（`audio` 権限のアプリが `audio.start` している間） */
  recording: boolean;
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

/** ホームの「アプリ一覧」の 1 行（Rust の `list_app_instances`） */
export interface AppInstanceListItem {
  worktreeId: string;
  worktreeName: string;
  artifactId: string;
  artifactTitle: string;
  /** manifest の表示名。manifest を読めない・アーティファクトが無いときは空 */
  appName: string;
  /** アーティファクトのファイルがある（承認だけが残っている行は false） */
  artifactExists: boolean;
  /** manifest を解析できなかった理由。起動はできない */
  manifestError: string | null;
  granted: boolean;
  grantedAt: number | null;
  resolved: AppResolvedPermissions | null;
  /** 起動処理中（承認ダイアログ待ち）/ 起動済み。停止中は null */
  state: "starting" | "running" | null;
  running: boolean;
  /** 起動中の権限変更が再承認待ち */
  reapprovalPending: boolean;
}

/** `list_app_instances` の戻り値 */
export interface AppInstanceList {
  items: AppInstanceListItem[];
  /** 承認ダイアログと同じく「ユーザーと同等の権限」として強調するツール */
  dangerousTools: string[];
}

/** 起動画面（ビューア）が使うアプリ操作。Tauri 実装だけが提供する（Web は起動不可） */
export interface AppLaunchApi {
  status(artifactId: string): Promise<AppInstanceStatus>;
  start(artifactId: string): Promise<void>;
  stop(artifactId: string): Promise<boolean>;
  /** 録音だけを止める（アプリは動かしたまま）。録音中でなければ false */
  stopRecording(artifactId: string): Promise<boolean>;
  toggleVisible(artifactId: string): Promise<boolean>;
  log(artifactId: string): Promise<AppLogEntry[]>;
  grant(artifactId: string): Promise<AppGrant | null>;
  revoke(artifactId: string): Promise<boolean>;
  onChanged(handler: (status: AppInstanceStatus) => void): Promise<() => void>;
  onLog(handler: (args: { artifactId: string; entry: AppLogEntry }) => void): Promise<() => void>;
}
