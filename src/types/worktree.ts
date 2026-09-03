import type { WorktreeEntry } from "./settings";
import type { AiSessionInfo } from "./terminal";

export interface WorktreeTerminal {
  id: number;
  title: string;
}

export interface Worktree extends WorktreeEntry {
  terminals: WorktreeTerminal[];
}

export interface SavedTerminal {
  title: string;
  buffer: string;
  /**
   * 保存時に動いていた AI エージェントのセッション情報 (#157)。
   * 復元時にインジケータを戻し、タブを初めて開いたときに resume コマンドを投入する。
   * 旧バージョンが書いた JSON には無いので undefined を素通りさせること。
   */
  aiSession?: AiSessionInfo;
}

export interface TerminalSessionFile {
  worktreeId: string;
  terminals: SavedTerminal[];
  savedAt: string;
}
