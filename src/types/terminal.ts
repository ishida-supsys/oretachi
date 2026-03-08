/** ターミナルエントリの基本型（SubWindowApp / FrameContainer / FramePane で共通） */
export interface SubTerminalEntry {
  id: number;
  title: string;
  sessionId: number;
  snapshot: string;
  isAiAgent?: boolean;
}

/** トレイポップアップ用ターミナルエントリ（PTYサイズ情報付き） */
export interface TrayTerminalEntry extends SubTerminalEntry {
  rows: number;
  cols: number;
}
