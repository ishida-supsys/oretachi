import type { WorktreeEntry } from "./settings";

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
}

export interface TerminalSessionFile {
  worktreeId: string;
  terminals: SavedTerminal[];
  savedAt: string;
}
