export interface Repository {
  id: string;
  name: string;
  path: string;
  execScript?: string; // 実行スクリプトの絶対パス
  copyTargets?: string[]; // .gitignoreから選択されたコピー対象エントリ
  packageManager?: string; // "npm" | "pnpm" | "yarn" | "bun" | undefined
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
}

export interface TerminalSettings {
  fontSize: number;
  shell?: string; // デフォルトシェル (空 = 各 OS のデフォルトにフォールバック)
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
}

export interface NotificationSoundSettings {
  volume: number;            // 0-100 (デフォルト: 80)
  approval?: string | null;  // null/"" = 音なし, "system:<filename>", "custom:<filename>"
  completed?: string | null;
  general?: string | null;
}

export interface AppSettings {
  repositories: Repository[];
  worktreeBaseDir: string;
  worktrees: WorktreeEntry[];
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
}
