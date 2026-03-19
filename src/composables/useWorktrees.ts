import { ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import type { Worktree, WorktreeTerminal, SavedTerminal, TerminalSessionFile } from "../types/worktree";
import type { WorktreeEntry } from "../types/settings";
import { useSettings } from "./useSettings";

const worktrees = ref<Worktree[]>([]);
let terminalCounter = 0;

const { settings, scheduleSave } = useSettings();

/** 永続化済みワークツリーエントリからランタイム用 Worktree を復元（ターミナルは空） */
function loadWorktreesFromSettings() {
  worktrees.value = settings.value.worktrees.map((entry) => ({
    ...entry,
    terminals: [],
  }));
}

/** ワークツリーを UI 一覧に仮追加（バックエンド処理前） */
function addWorktreePlaceholder(entry: WorktreeEntry): void {
  const worktree: Worktree = { ...entry, terminals: [] };
  worktrees.value.push(worktree);
}

/** ワークツリーをバックエンドで作成（git worktree add） */
async function invokeWorktreeAdd(entry: WorktreeEntry): Promise<boolean> {
  return invoke<boolean>("git_worktree_add", {
    repoPath: entry.repositoryId,
    worktreePath: entry.path,
    branchName: entry.branchName,
  });
}

/** ワークツリーを設定に永続化（成功時に呼ぶ） */
function commitWorktree(entry: WorktreeEntry): void {
  settings.value.worktrees.push(entry);
  scheduleSave();
}

/** 仮追加したワークツリーを一覧から削除（失敗時ロールバック用） */
function rollbackWorktree(worktreeId: string): void {
  const index = worktrees.value.findIndex((w) => w.id === worktreeId);
  if (index !== -1) {
    worktrees.value.splice(index, 1);
  }
}

interface RemoveWorktreeOptions {
  mergeTo?: string;
  deleteBranch?: boolean;
  forceBranch?: boolean;
}

/** ワークツリーを削除（git worktree remove + 設定から削除） */
async function removeWorktree(worktreeId: string, options?: RemoveWorktreeOptions): Promise<void> {
  const index = worktrees.value.findIndex((w) => w.id === worktreeId);
  if (index === -1) return;

  const worktree = worktrees.value[index];

  // リポジトリパスを取得（id = リポジトリパスとして使用）
  const repoEntry = settings.value.repositories.find(
    (r) => r.id === worktree.repositoryId
  );
  if (repoEntry) {
    if (options?.mergeTo) {
      await invoke("git_merge_branch", {
        repoPath: repoEntry.path,
        sourceBranch: worktree.branchName,
        targetBranch: options.mergeTo,
      });
    }

    await invoke("git_worktree_remove", {
      repoPath: repoEntry.path,
      worktreePath: worktree.path,
    });

    if (options?.deleteBranch) {
      await invoke("git_delete_branch", {
        repoPath: repoEntry.path,
        branchName: worktree.branchName,
        force: options.forceBranch ?? false,
      });
    }
  }

  worktrees.value.splice(index, 1);
  settings.value.worktrees = settings.value.worktrees.filter(
    (w) => w.id !== worktreeId
  );
  scheduleSave();

  // セッションファイルを削除
  try {
    await invoke("delete_terminal_session", { worktreeId });
  } catch {
    // ファイルが存在しない場合は無視
  }

  // アーティファクトを削除
  try {
    await invoke("delete_artifacts", { worktreeId });
  } catch {
    // ディレクトリが存在しない場合は無視
  }
}

/** ローカルブランチ一覧を取得 */
async function listBranches(repositoryId: string): Promise<string[]> {
  const repoEntry = settings.value.repositories.find((r) => r.id === repositoryId);
  if (!repoEntry) return [];
  return invoke<string[]>("git_list_branches", { repoPath: repoEntry.path });
}

/** ターミナルを追加 */
function addTerminal(worktreeId: string): WorktreeTerminal {
  const worktree = worktrees.value.find((w) => w.id === worktreeId);
  if (!worktree) throw new Error(`Worktree ${worktreeId} not found`);

  terminalCounter++;
  const terminal: WorktreeTerminal = {
    id: terminalCounter,
    title: `Terminal ${worktree.terminals.length + 1}`,
  };
  worktree.terminals.push(terminal);
  return terminal;
}

/** ターミナルを削除 */
function removeTerminal(worktreeId: string, terminalId: number) {
  const worktree = worktrees.value.find((w) => w.id === worktreeId);
  if (!worktree) return;

  const idx = worktree.terminals.findIndex((t) => t.id === terminalId);
  if (idx !== -1) {
    worktree.terminals.splice(idx, 1);
  }
}

/** ターミナルセッションを保存 */
async function saveTerminalSession(worktreeId: string, terminals: SavedTerminal[]): Promise<void> {
  const sessionFile: TerminalSessionFile = {
    worktreeId,
    terminals,
    savedAt: new Date().toISOString(),
  };
  await invoke("save_terminal_session", {
    worktreeId,
    dataJson: JSON.stringify(sessionFile),
  });
}

/** ターミナルセッションを読み込み */
async function loadTerminalSession(worktreeId: string): Promise<TerminalSessionFile | null> {
  const json = await invoke<string | null>("load_terminal_session", { worktreeId });
  if (!json) return null;
  try {
    return JSON.parse(json) as TerminalSessionFile;
  } catch {
    return null;
  }
}

/** ターミナルセッションを削除 */
async function deleteTerminalSession(worktreeId: string): Promise<void> {
  await invoke("delete_terminal_session", { worktreeId });
}

/** ターミナルタイトルを更新 */
function updateTerminalTitle(worktreeId: string, terminalId: number, title: string) {
  const worktree = worktrees.value.find((w) => w.id === worktreeId);
  if (!worktree) return;
  const terminal = worktree.terminals.find((t) => t.id === terminalId);
  if (!terminal) return;
  terminal.title = title;
}

export function useWorktrees() {
  return {
    worktrees,
    loadWorktreesFromSettings,
    addWorktreePlaceholder,
    invokeWorktreeAdd,
    commitWorktree,
    rollbackWorktree,
    removeWorktree,
    listBranches,
    addTerminal,
    removeTerminal,
    updateTerminalTitle,
    saveTerminalSession,
    loadTerminalSession,
    deleteTerminalSession,
  };
}
