import { ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import type { Worktree, WorktreeTerminal } from "../types/worktree";
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

/** ワークツリーを作成して設定に保存 */
async function addWorktree(entry: WorktreeEntry): Promise<boolean> {
  const lfsSkipped = await invoke<boolean>("git_worktree_add", {
    repoPath: entry.repositoryId, // repositoryId にリポジトリパスを使用
    worktreePath: entry.path,
    branchName: entry.branchName,
  });

  const worktree: Worktree = { ...entry, terminals: [] };
  worktrees.value.push(worktree);

  settings.value.worktrees.push(entry);
  scheduleSave();

  return lfsSkipped;
}

interface RemoveWorktreeOptions {
  mergeTo?: string;
  deleteBranch?: boolean;
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
      });
    }
  }

  worktrees.value.splice(index, 1);
  settings.value.worktrees = settings.value.worktrees.filter(
    (w) => w.id !== worktreeId
  );
  scheduleSave();
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
    addWorktree,
    removeWorktree,
    listBranches,
    addTerminal,
    removeTerminal,
    updateTerminalTitle,
  };
}
