/** 通知種別 兼 購読イベント種別の固定7値（issue #140）。
 *
 *  Rust 側の `event_db::NotifyKind::ALL` と**同じ並び・同じ文字列**であること。
 *  両側に pin テスト（`notificationKinds.test.ts` / `test_notify_kind_all_seven_values_pinned`）
 *  があるので、片方だけ変えると必ずどちらかが落ちる。
 *
 *  接頭辞ありとなしが混在しているのは、`kind` の値が settings.json と各ワークツリーの
 *  `.claude/settings.local.json` および events.db に既に焼き付いているため。揃えるには
 *  全面的な移行処理が要るので、混在を許容して移行ゼロを選んでいる。 */
export const NOTIFY_KINDS = [
  'hook',
  'approval',
  'completed',
  'general',
  'worktree.message',
  'worktree.created',
  'worktree.closed',
] as const;
export type NotifyKind = (typeof NOTIFY_KINDS)[number];

/** Claude Code のライフサイクルフックが名乗れる種別。
 *
 *  **7値のうち4値だけ。** ここを広げると、設定1行でフックの JSON が
 *  `worktree.message` として他ワークツリーへ自由文配送される
 *  （Rust 側 `NotifyKind::allowed_as_hook_entry` と対応）。 */
export const HOOK_NOTIFY_KINDS = ['hook', 'approval', 'completed', 'general'] as const;
export type HookNotifyKind = (typeof HOOK_NOTIFY_KINDS)[number];

export interface NotificationHookEntry {
  event: 'Stop' | 'Notification' | 'SubagentStop' | 'PreToolUse' | 'PostToolUse' | 'PermissionRequest';
  kind: HookNotifyKind;
}

export interface Repository {
  id: string;
  name: string;
  path: string;
  execScript?: string; // 実行スクリプトの絶対パス
  copyTargets?: string[]; // .gitignoreから選択されたコピー対象エントリ
  packageManager?: string; // "npm" | "pnpm" | "yarn" | "bun" | undefined
  packageManagerArgs?: string; // install コマンドに追加する引数 (例: --config.node-linker=hoisted)
  notificationHooks?: NotificationHookEntry[]; // Claude Code通知フック設定
  pullBeforeAdd?: boolean; // ワークツリー追加前に git pull を実行するか
  branchNamePattern?: string; // タスク追加時のブランチ名パターン (例: "{feature|fix}/<task>"、未記入なら "worktree/<task>")
}

export interface WorktreeEntry {
  id: string;
  name: string;
  repositoryId: string;
  repositoryName: string;
  path: string;
  branchName: string;
  hotkeyChar?: string; // Alt+[この文字] でフォーカス
  autoApproval?: boolean;
  autoApprovalPrompt?: string;
  /**
   * フック由来通知をトレイ通知として出すか。未設定 = true（解決はバックエンドの
   * resolve_tray_notification / フロントの resolveTrayNotification）。所属ワークグループの
   * 既定値は**作成時に一度だけ焼き込まれる**ので、実効値を決めるのはここだけ。
   * false でもイベント自体は流れるため自動承認は動き、
   * MCP notify_worktree による明示的な通知も常にトレイへ出る。
   */
  trayNotification?: boolean;
  description?: string; // 作業全体の目的を表す1行説明（ExitPlanMode hookのAI要約、または MCP oretachi_set_description で直接セット）
  descriptionOpen?: boolean; // ホームカードの description 開閉状態（ワークツリー毎）
  workgroupId?: string; // 所属するワークグループのID（未設定は先頭グループにフォールバック）
  /**
   * ホームワークツリー。path = worktreeBaseDir を作業ディレクトリとする擬似ワークツリーで、
   * git ワークツリーではないため削除・複製・マージ・ブランチ操作を一切通さない。
   * 一覧では常に先頭に固定される（utils/sortHomeFirst）。
   */
  isHome?: boolean;
  /**
   * リポジトリ擬似ワークツリー。path = Repository.path を作業ディレクトリとする擬似ワークツリーで、
   * git ワークツリーではないため削除・複製・マージ・ブランチ操作を一切通さない。
   * settings.repositories を正として migrateRepositoryWorktrees が生成・追従・prune する
   * （utils/repositoryWorktree）。
   */
  isRepository?: boolean;
}

// Claude Code の起動モード（taskAddAgent が claudeCode のときのみ意味を持つ）
export type ClaudeCodeMode = 'plan' | 'manual' | 'acceptEdit' | 'auto';

export interface Workgroup {
  id: string;
  name?: string;                    // 未指定時は表示時に「グループ(番号)」を生成
  color?: string;                   // プリセット色。未指定 = 無色
  autoAssignHotkey?: boolean;       // ホットキー自動割り当て（グループ単位）
  autoReturnHomeAfterTask?: boolean; // タスク完了後、メインウィンドウが非フォーカスのまま5秒経過したらホームタブへ自動復帰（グループ単位、既定 OFF）
  taskAddAgent?: AiAgentKind;       // タスク実行エージェント（グループ単位）
  claudeCodeMode?: ClaudeCodeMode;  // Claude Code モード（既定: plan）
  execPrompt?: string;              // 実行プロンプトテンプレート（置換タグ {{PROMPT}}）
  systemPrompt?: string;            // Claude Code セッションに常時注入（SessionStart フック経由。/clear 後も維持）
  trayNotification?: boolean;       // 新規ワークツリー作成時のトレイ通知初期値。既存ワークツリーには影響しない（未設定 = 焼き込まない = 実効値 true）
}

export interface TerminalSettings {
  fontSize: number;
  shell?: string; // デフォルトシェル (空 = 各 OS のデフォルトにフォールバック)
  /** MCP 等で起動する背景ペインの分割方向 (デフォルト: bottom) */
  backgroundPaneSplitDirection?: "left" | "right" | "top" | "bottom";
}

export interface HotkeyBinding {
  ctrl?: boolean;
  meta?: boolean;
  shift?: boolean;
  alt?: boolean;
  key: string; // KeyboardEvent.key の値 (例: "Tab", "t", "q")
}

export interface HotkeySettings {
  globalTrayPopup: HotkeyBinding;
  terminalNext: HotkeyBinding;  // デフォルト: { ctrl: true, key: "Tab" }
  terminalPrev: HotkeyBinding;  // デフォルト: { ctrl: true, shift: true, key: "Tab" }
  terminalAdd: HotkeyBinding;   // デフォルト: { ctrl: true, key: "t" }
  terminalClose: HotkeyBinding; // デフォルト: { ctrl: true, key: "q" }
  trayNext: HotkeyBinding;      // デフォルト: { ctrl: true, key: "n" }
  homeTab: HotkeyBinding;         // デフォルト: { alt: true, key: "0" }
  addTask: HotkeyBinding;       // デフォルト: { ctrl: true, shift: true, key: "n" }
  workgroupNext: HotkeyBinding; // デフォルト: { ctrl: true, key: "PageDown" }
  workgroupPrev: HotkeyBinding; // デフォルト: { ctrl: true, key: "PageUp" }
}

export type AiAgentKind = 'claudeCode' | 'geminiCli' | 'codexCli' | 'clineCli';

export interface AiAgentSettings {
  approvalAgent?: AiAgentKind;
  taskAddAgent?: AiAgentKind;
  remoteExec?: boolean;
}

export interface WorktreeDefaults {
  openInSubWindow?: boolean;
  autoApproval?: boolean;
  autoOpenArtifact?: boolean;
}

export interface CodeReviewSettings {
  monacoFontSize?: number;          // デフォルト: 13
  monacoMinimap?: boolean;          // デフォルト: true
  monacoWordWrap?: 'on' | 'off';    // デフォルト: 'off'
  monacoLineNumbers?: 'on' | 'off'; // デフォルト: 'on'
  chatHotkey?: HotkeyBinding;       // デフォルト: { ctrl: true, key: 'l' }
  autoOpenReviewOnDiff?: boolean;   // デフォルト: true
}

export interface AppearanceSettings {
  enableAcrylic?: boolean; // デフォルト: true
  acrylicOpacity?: number; // 0-255, デフォルト: backdrop=125, blur=240
  acrylicColor?: string;   // "#RRGGBB", デフォルト: "#121212"
  enableGamingBorder?: boolean; // デフォルト: false
  gamingBorderTheme?: string;   // デフォルト: 'gaming'
  uiScale?: 'normal' | 'large' | 'xlarge'; // デフォルト: 'normal'
}

/** 通知種別ごとの通知設定（#140）。 */
export interface NotificationKindSetting {
  /** false ならこの種別の通知（バッジ / 音 / OS 通知）を一切出さない */
  enabled: boolean;
  /** null/"" = 音なし, "system:<filename>", "custom:<filename>" */
  sound?: string | null;
  /** OS 通知を出すか。未指定なら enabled に従う */
  os?: boolean;
}

export interface NotificationSoundSettings {
  volume: number;            // 0-100 (デフォルト: 80)
  /** 種別 → 設定。キーは NOTIFY_KINDS の7値。
   *
   *  `worktree.message` のようにドットを含む種別があるので、フラットなキーには
   *  できない（旧形式は下の deprecated フィールドから migrateNotificationSound が畳む）。 */
  kinds?: Partial<Record<NotifyKind, NotificationKindSetting>>;
  /** @deprecated #140 以前のフラット形式。`migrateNotificationSound` が `kinds` へ畳んで消す */
  approval?: string | null;
  /** @deprecated #140 以前のフラット形式 */
  completed?: string | null;
  /** @deprecated #140 以前のフラット形式 */
  general?: string | null;
}

export interface AppSettings {
  repositories: Repository[];
  worktreeBaseDir: string;
  worktrees: WorktreeEntry[];
  workgroups?: Workgroup[];
  activeWorkgroupId?: string; // ホームで選択中のワークグループ
  terminal: TerminalSettings;
  hotkeys: HotkeySettings;
  alwaysOnTop: boolean;
  enableOsNotification?: boolean;
  autoAssignHotkey?: boolean;
  detachedWorktreeIds?: string[];
  focusMainOnEmptyTray?: boolean;
  aiAgent?: AiAgentSettings;
  worktreeDefaults?: WorktreeDefaults;
  locale?: string;
  codeReview?: CodeReviewSettings;
  appearance?: AppearanceSettings;
  notificationSound?: NotificationSoundSettings;
  mcpPort?: number;
  mcpApiKey?: string;
  mcpRemoteAccess?: boolean;
  enableHomeCat?: boolean;
  aiTimeoutSecs?: number; // AIタイムアウト秒数 (デフォルト: 120)
  debugMode?: boolean;
  useOretachiTerminalForBackground?: boolean; // AI からの background コマンドを oretachi ターミナルで起動するか (デフォルト: false)
  moveToSubWindowOnMcpSpawn?: boolean; // MCP 経由のターミナル追加時にサブウィンドウへ自動移行するか (デフォルト: false)
  homeAgentPrompt?: string; // home のセッションに SessionStart で注入するプロンプト (空なら Rust 側の既定値)
  wizardCompleted?: boolean; // 初回起動ウィザード完了フラグ (Rust 側 init() でシーディング)
  // trayNotification 移行フラグ (#171)。グループ既定値を既存ワークツリーへ一度だけ焼き込んだか。
  // 一度きりを保証するために永続化する（migrateTrayNotification のコメント参照）
  trayNotificationMigrated?: boolean;
}
