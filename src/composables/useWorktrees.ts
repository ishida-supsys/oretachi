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
async function invokeWorktreeAdd(entry: WorktreeEntry, sourceBranch?: string): Promise<boolean> {
  return invoke<boolean>("git_worktree_add", {
    repoPath: entry.repositoryId,
    worktreePath: entry.path,
    branchName: entry.branchName,
    sourceBranch: sourceBranch ?? null,
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
async function removeWorktree(worktreeId: string, options?: RemoveWorktreeOptions, onBeforeSplice?: () => Promise<void>): Promise<void> {
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

    // git_worktree_remove 成功済み: ブランチ削除を試みる。
    // 失敗した場合はワークツリーを復元してエラーを即座に伝播し、UI から消さない。
    // 復元自体も失敗した場合のみ、ディレクトリが既に消えているため従来通りクリーンアップを続行する。
    let branchDeleteError: unknown = null;
    if (options?.deleteBranch) {
      try {
        await invoke("git_delete_branch", {
          repoPath: repoEntry.path,
          branchName: worktree.branchName,
          force: options.forceBranch ?? false,
        });
      } catch (e) {
        // ワークツリーを復元して、削除前の状態に戻す
        let restored = false;
        try {
          await invoke("git_worktree_restore", {
            repoPath: repoEntry.path,
            worktreePath: worktree.path,
            branchName: worktree.branchName,
          });
          restored = true;
        } catch {
          // 復元失敗: ディレクトリは既に消えているため従来通りクリーンアップして続行
        }
        if (restored) {
          // 復元成功: ワークツリーは元に戻ったのでエラーをそのまま throw（UI から消さない）
          throw e;
        }
        branchDeleteError = e;
      }
    }

    if (onBeforeSplice) await onBeforeSplice();
    worktrees.value.splice(index, 1);
    settings.value.worktrees = settings.value.worktrees.filter(
      (w) => w.id !== worktreeId
    );
    scheduleSave();

    // セッションファイル・アーティファクトのクリーンアップ（ブランチ削除失敗時も実行）
    try { await invoke("delete_terminal_session", { worktreeId }); } catch { /* 存在しない場合は無視 */ }
    try { await invoke("delete_artifacts", { worktreeId }); } catch { /* 存在しない場合は無視 */ }

    // クリーンアップ完了後にブランチ削除エラーを伝播
    if (branchDeleteError) throw branchDeleteError;
    return;
  }

  if (onBeforeSplice) await onBeforeSplice();

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

/** ワークツリーの順序を変更（fromId を toId の位置に移動）。保存は行わない */
function reorderWorktree(fromId: string, toId: string): void {
  if (fromId === toId) return;

  const fromIdx = worktrees.value.findIndex((w) => w.id === fromId);
  const toIdx = worktrees.value.findIndex((w) => w.id === toId);
  if (fromIdx === -1 || toIdx === -1) return;

  const [item] = worktrees.value.splice(fromIdx, 1);
  worktrees.value.splice(toIdx, 0, item);

  const fromSettingsIdx = settings.value.worktrees.findIndex((w) => w.id === fromId);
  const toSettingsIdx = settings.value.worktrees.findIndex((w) => w.id === toId);
  if (fromSettingsIdx !== -1 && toSettingsIdx !== -1) {
    const [settingsItem] = settings.value.worktrees.splice(fromSettingsIdx, 1);
    settings.value.worktrees.splice(toSettingsIdx, 0, settingsItem);
  }
}

/** 現在の worktrees 順序を設定に保存する */
function saveWorktreeOrder(): void {
  scheduleSave();
}

/** ワークツリーの順序を指定された ID 順に復元する。スナップショット外の ID は末尾に追加 */
function restoreWorktreeOrder(orderIds: string[]): void {
  const orderSet = new Set(orderIds);

  const sorted = orderIds
    .map((id) => worktrees.value.find((w) => w.id === id))
    .filter((w): w is typeof worktrees.value[number] => w !== undefined);
  const extra = worktrees.value.filter((w) => !orderSet.has(w.id));
  worktrees.value = [...sorted, ...extra];

  const sortedSettings = orderIds
    .map((id) => settings.value.worktrees.find((w) => w.id === id))
    .filter((w): w is typeof settings.value.worktrees[number] => w !== undefined);
  const extraSettings = settings.value.worktrees.filter((w) => !orderSet.has(w.id));
  settings.value.worktrees = [...sortedSettings, ...extraSettings];
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
    reorderWorktree,
    saveWorktreeOrder,
    restoreWorktreeOrder,
    listBranches,
    addTerminal,
    removeTerminal,
    updateTerminalTitle,
    saveTerminalSession,
    loadTerminalSession,
  };
}
