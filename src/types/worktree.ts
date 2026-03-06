import type { WorktreeEntry } from "./settings";

export interface WorktreeTerminal {
  id: number;
  title: string;
}

export interface Worktree extends WorktreeEntry {
  terminals: WorktreeTerminal[];
}
