export interface Repository {
  id: string;
  name: string;
  path: string;
  execScript?: string; // 実行スクリプトの絶対パス
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
  focusMainWindow: HotkeyBinding; // デフォルト: { alt: true, key: "m" }
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
}
