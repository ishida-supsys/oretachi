import type { InjectionKey } from "vue";
import type { ArtifactMeta, ArtifactData, ArtifactState, CopyArtifactResult } from "../types/artifact";

/**
 * ArtifactViewerApp.vue が使うデータアクセス / プラットフォーム操作の抽象層。
 *
 * Tauri 実装（tauriArtifactDataSource.ts）が現行の挙動をそのまま提供する。
 * Web 版では HTTP + SSE 実装を後続の sub-issue で用意する想定で、
 * このファイル自体はどちらのランタイムにも依存しない（純粋な型定義のみ）。
 */

export type ArtifactScopeKind = "worktree" | "repository";

/** このビューアが開いているスコープ。両方の ID を持ち、使う側は kind で選ぶ */
export interface ArtifactScopeRef {
  kind: ArtifactScopeKind;
  worktreeId: string;
  repositoryId: string;
}

/**
 * Web 版では提供しない操作を UI ごと隠すためのフラグ。
 * Tauri 実装は全て true を返す（挙動を変えないため）。
 */
export interface ArtifactCapabilities {
  pin: boolean;
  lock: boolean;
  delete: boolean;
  export: boolean;
  import: boolean;
  copyPng: boolean;
  transfer: boolean;
}

export interface ArtifactChangedHandlerArgs {
  artifactId: string;
  command: string;
  /**
   * 作業の副産物として作られたか（フックによる URL アーティファクトの自動登録など）。
   * false のときはユーザーの作業に割り込まない。
   */
  autoOpen: boolean;
}

export interface ArtifactStateChangedHandlerArgs {
  artifactId: string;
}

export interface ArtifactDataSource {
  scope: ArtifactScopeRef;
  capabilities: ArtifactCapabilities;

  list(): Promise<ArtifactMeta[]>;
  read(artifactId: string): Promise<ArtifactData>;
  listStates(): Promise<Record<string, ArtifactState>>;
  resolveScopeName(): Promise<{ displayName: string; repositoryName: string | null }>;

  setMemory(artifactId: string, memory: Record<string, unknown> | null): Promise<void>;
  callMcpTool(artifactId: string, tool: string, params: Record<string, unknown>): Promise<string>;

  /** capabilities.pin が true のときのみ利用可能 */
  setPinned?(artifactId: string, pinned: boolean): Promise<void>;

  /** capabilities.lock が true のときのみ利用可能 */
  lockHeartbeatIntervalMs?(): Promise<number>;
  touchLock?(artifactId: string): Promise<void>;
  releaseLock?(): Promise<void>;

  /** capabilities.delete が true のときのみ利用可能 */
  deleteArtifact?(artifactId: string): Promise<void>;

  /** capabilities.export が true のときのみ利用可能 */
  exportArtifact?(
    artifactId: string,
    destPath: string,
    viewHtml: string | null,
  ): Promise<{ path: string; bytes: number }>;

  /** capabilities.import が true のときのみ利用可能 */
  importArtifact?(
    zipPath: string,
  ): Promise<{ artifactId: string; title: string; renamedFrom: string | null }>;

  /** capabilities.transfer が true のときのみ利用可能 */
  copyToRepository?(artifactId: string, overwrite: boolean): Promise<CopyArtifactResult>;

  /** イベント購読。戻り値は購読解除関数（このスコープ宛でないイベントは実装側で捨てる） */
  onArtifactChanged(handler: (args: ArtifactChangedHandlerArgs) => void): Promise<() => void>;
  onStateChanged(handler: (args: ArtifactStateChangedHandlerArgs) => void): Promise<() => void>;
}

/** データ以外のプラットフォーム操作（ダイアログ・クリップボード・ウィンドウ・外部リンク） */
export interface ArtifactViewerHost {
  confirm(message: string, opts: { title: string; kind: "warning" | "info" | "error" }): Promise<boolean>;
  showError(message: string, title: string): Promise<void>;
  /** 保存先を尋ねる。キャンセルなら null */
  pickSavePath(defaultName: string): Promise<string | null>;
  /** 取り込む zip を尋ねる。キャンセルなら null */
  pickZipToOpen(): Promise<string | null>;
  writeImage(bytes: Uint8Array): Promise<void>;
  setWindowTitle(title: string): Promise<void>;
  /** http(s) の外部リンクを既定のブラウザで開く */
  openExternalUrl(url: string): Promise<void>;
  /** 別スコープのアーティファクトビューアを開く（新規ウィンドウ or 既存へ遷移指示） */
  openScopeViewer(scope: ArtifactScopeKind, id: string, artifactId: string): Promise<void>;
  /** 既存ウィンドウへの「このアーティファクトを選べ」という遷移指示を購読する */
  onNavigate(handler: (artifactId: string) => void): Promise<() => void>;
}

/** components/artifact/* から inject して使う（openExternalUrl / confirm） */
export const ARTIFACT_VIEWER_HOST_KEY: InjectionKey<ArtifactViewerHost> = Symbol("artifactViewerHost");
