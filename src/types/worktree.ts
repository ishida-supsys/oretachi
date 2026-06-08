import type { WorktreeEntry } from "./settings";

export interface WorktreeTerminal {
  id: number;
  title: string;
}

export interface Worktree extends WorktreeEntry {
  terminals: WorktreeTerminal[];
  // description の正本は worktree_descriptions DB。ランタイムでは DB からロードして保持する
  description?: string;
}

export interface SavedTerminal {
  title: string;
  buffer: string;
}

export interface TerminalSessionFile {
  worktreeId: string;
  terminals: SavedTerminal[];
  savedAt: string;
}
