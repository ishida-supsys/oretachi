import type { WorktreeEntry } from "../types/settings";

/**
 * ホームワークツリーの固定 ID。
 * terminal-sessions/<id>.json のファイル名にもなるため、パス成分として安全な値に固定する。
 */
export const HOME_WORKTREE_ID = "home";

/** ホームワークツリーかどうか */
export function isHomeWorktree(worktree: { isHome?: boolean } | undefined | null): boolean {
  return worktree?.isHome === true;
}

/**
 * ホームを先頭に固定した新しい配列を返す（ホーム以外の相対順序は維持）。
 * ホームが無い場合は元の順序のまま。
 */
export function sortHomeFirst<T extends { isHome?: boolean }>(worktrees: T[]): T[] {
  if (!worktrees.some(isHomeWorktree)) return worktrees;
  return [...worktrees.filter(isHomeWorktree), ...worktrees.filter((w) => !isHomeWorktree(w))];
}

/** ワークツリー追加先ディレクトリを作業ディレクトリとするホームエントリを生成する */
export function makeHomeWorktreeEntry(worktreeBaseDir: string): WorktreeEntry {
  return {
    id: HOME_WORKTREE_ID,
    name: "home",
    repositoryId: "",
    repositoryName: "",
    path: worktreeBaseDir,
    branchName: "",
    isHome: true,
  };
}
