import type { Repository, WorktreeEntry } from "../types/settings";
import { isHomeWorktree } from "./homeWorktree";

/**
 * リポジトリ擬似ワークツリーの ID 接頭辞。
 * terminal-sessions/<id>.json のファイル名にもなるため、パス成分として安全な値だけを使う。
 */
export const REPOSITORY_WORKTREE_ID_PREFIX = "repo-";

/** ファイル名に使えない文字を潰す */
function sanitizeSegment(value: string): string {
  return value.replace(/[^A-Za-z0-9._-]/g, "_").slice(0, 32);
}

/** FNV-1a 32bit。依存を増やさず同期的に安定ハッシュを得るために使う */
function fnv1a32(value: string): string {
  let hash = 0x811c9dc5;
  for (let i = 0; i < value.length; i++) {
    hash ^= value.charCodeAt(i);
    // 32bit の FNV prime 乗算。Math.imul で桁あふれを 32bit に丸める
    hash = Math.imul(hash, 0x01000193);
  }
  return (hash >>> 0).toString(16).padStart(8, "0");
}

/** パス末尾のセグメント（区切りは / と \ の両方を許容し、末尾の区切りは無視する） */
function basename(path: string): string {
  const segments = path.split(/[\\/]+/).filter((s) => s.length > 0);
  return segments[segments.length - 1] ?? "";
}

/**
 * repository.id（絶対パス）からリポジトリ擬似ワークツリーの ID を導出する。
 *
 * 決定論的にしているのは、メイン/サブウィンドウが同時に settings-changed を受けて
 * マイグレーションを走らせても同じ ID になり二重登録が起きないようにするため。
 * また、リポジトリ削除後にセッションファイルを掃除する際、settings に残骸が無くても
 * ID を再計算できる。
 */
export function makeRepositoryWorktreeId(repositoryId: string): string {
  const label = sanitizeSegment(basename(repositoryId)) || "repo";
  return `${REPOSITORY_WORKTREE_ID_PREFIX}${label}-${fnv1a32(repositoryId)}`;
}

/** リポジトリ擬似ワークツリーかどうか */
export function isRepositoryWorktree(
  worktree: { isRepository?: boolean } | undefined | null,
): boolean {
  return worktree?.isRepository === true;
}

/** ホーム / リポジトリのいずれかの擬似ワークツリーか（git 操作を通してはいけないもの） */
export function isPseudoWorktree(
  worktree: { isHome?: boolean; isRepository?: boolean } | undefined | null,
): boolean {
  return isHomeWorktree(worktree) || isRepositoryWorktree(worktree);
}

/** リポジトリのルートを作業ディレクトリとする擬似ワークツリーエントリを生成する */
export function makeRepositoryWorktreeEntry(repository: Repository): WorktreeEntry {
  return {
    id: makeRepositoryWorktreeId(repository.id),
    name: repository.name,
    repositoryId: repository.id,
    repositoryName: repository.name,
    path: repository.path,
    // git ワークツリーではないのでブランチは持たない。
    // WorktreeCard / WorktreeHeader / RemoveWorktreeDialog がここを起点に git 操作 UI を出すため、
    // 空文字であることが実質的なガードになっている。
    branchName: "",
    isRepository: true,
  };
}

/**
 * 擬似ワークツリー（ホーム → リポジトリ）を先頭に固定した新しい配列を返す。
 * 同種内・通常ワークツリー内の相対順序は維持する。擬似エントリが無い場合は元の配列をそのまま返す。
 */
export function sortPseudoFirst<T extends { isHome?: boolean; isRepository?: boolean }>(
  worktrees: T[],
): T[] {
  if (!worktrees.some(isPseudoWorktree)) return worktrees;
  return [
    ...worktrees.filter(isHomeWorktree),
    ...worktrees.filter(isRepositoryWorktree),
    ...worktrees.filter((w) => !isPseudoWorktree(w)),
  ];
}
