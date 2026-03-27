import { ref, nextTick } from "vue";
import type { Ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { message } from "@tauri-apps/plugin-dialog";
import { useWorktrees } from "./useWorktrees";
import { cancelApproval } from "../utils/autoApproval";
import { saveArchive, deleteArchive } from "./useArchivePersistence";
import type { Worktree } from "../types/worktree";

interface TerminalRef {
  isRunning: boolean;
  kill(): Promise<void>;
}

interface HomeViewRef {
  fadeOutCard(worktreeId: string): Promise<void>;
  hideCard(worktreeId: string): Map<string, DOMRect>;
  animateAfterRemove(positions: Map<string, DOMRect>): void;
  unhideCard(worktreeId: string): void;
}

interface FrameBundle {
  terminalRefs: { get(id: number): TerminalRef | undefined };
}

type RemoveOptions = { mergeTo: string; deleteBranch: boolean; forceBranch: boolean };

export function useRemoveWorktreeDialog(options: {
  loadingWorktrees: Map<string, string>;
  clearNotification: (id: string) => void;
  isDetached: (id: string) => boolean;
  moveToMainWindow: (id: string) => Promise<void>;
  subWindowFocusMap: { delete(key: string): void };
  closeArtifactWindow: (id: string) => Promise<void>;
  worktreeFrameBundles: { get(id: string): FrameBundle | undefined; delete(id: string): void };
  getTerminalRef: (id: number) => TerminalRef | undefined;
  terminalWorktreeMap: { delete(key: number): void };
  activeWorktreeId: Ref<string | null>;
  goHome: () => void;
  homeViewRef: Ref<HomeViewRef | null>;
  t: (key: string, params?: Record<string, unknown>) => string;
}) {
  const {
    loadingWorktrees,
    clearNotification,
    isDetached,
    moveToMainWindow,
    subWindowFocusMap,
    closeArtifactWindow,
    worktreeFrameBundles,
    getTerminalRef,
    terminalWorktreeMap,
    activeWorktreeId,
    goHome,
    homeViewRef,
    t,
  } = options;

  const { worktrees, removeWorktree, listBranches } = useWorktrees();

  // ダイアログ state
  const showRemoveDialog = ref(false);
  const removeTargetWorktree = ref<{ id: string; name: string; branchName: string } | null>(null);
  const removeBranches = ref<string[]>([]);
  const removeDirtyFiles = ref<{ path: string; status: string; staged: boolean }[]>([]);

  /** 削除/アーカイブダイアログを開く */
  async function onRemoveWorktree(worktreeId: string) {
    clearNotification(worktreeId);
    const worktree = worktrees.value.find((w) => w.id === worktreeId);
    if (!worktree) return;

    const [branches, dirtyFiles] = await Promise.all([
      listBranches(worktree.repositoryId)
        .then((all) => all.filter((b) => b !== worktree.branchName))
        .catch(() => [] as string[]),
      invoke<{ path: string; status: string; staged: boolean }[]>("git_get_status", {
        repoPath: worktree.path,
      }).catch(() => [] as { path: string; status: string; staged: boolean }[]),
    ]);

    removeTargetWorktree.value = { id: worktree.id, name: worktree.name, branchName: worktree.branchName };
    removeBranches.value = branches;
    removeDirtyFiles.value = dirtyFiles;
    showRemoveDialog.value = true;
  }

  function dismissDialog() {
    showRemoveDialog.value = false;
    removeDirtyFiles.value = [];
  }

  /** 共通: ダイアログ後処理・ターミナル停止・アニメーション付きワークツリー削除
   *  beforeRemove: git 操作前に呼ぶ任意の非同期処理（アーカイブ保存など）
   *  onRemoveFailed: git 操作失敗時に beforeRemove の副作用をロールバックするコールバック
   */
  async function _confirm(
    removeOptions: RemoveOptions,
    loadingText: string,
    beforeRemove?: (worktree: Worktree) => Promise<void>,
    onRemoveFailed?: (worktree: Worktree) => Promise<void>,
  ): Promise<void> {
    if (!removeTargetWorktree.value) return;
    const { id: worktreeId } = removeTargetWorktree.value;

    const worktree = worktrees.value.find((w) => w.id === worktreeId);
    if (!worktree) {
      showRemoveDialog.value = false;
      removeDirtyFiles.value = [];
      return;
    }

    showRemoveDialog.value = false;
    removeTargetWorktree.value = null;
    removeBranches.value = [];
    removeDirtyFiles.value = [];
    clearNotification(worktreeId);
    loadingWorktrees.set(worktreeId, loadingText);
    try {
      if (isDetached(worktreeId)) {
        await moveToMainWindow(worktreeId);
        subWindowFocusMap.delete(worktreeId);
      }

      await closeArtifactWindow(worktreeId);
      await cancelApproval(worktreeId);

      const bundle = worktreeFrameBundles.get(worktreeId);
      for (const terminal of [...worktree.terminals]) {
        const term = bundle?.terminalRefs.get(terminal.id) ?? getTerminalRef(terminal.id);
        if (term?.isRunning) await term.kill();
        terminalWorktreeMap.delete(terminal.id);
      }
      worktreeFrameBundles.delete(worktreeId);

      if (activeWorktreeId.value === worktreeId) goHome();

      // git 操作前に事前処理（アーカイブ保存など）を実行
      if (beforeRemove) await beforeRemove(worktree);

      let savedPositions: Map<string, DOMRect> | undefined;
      try {
        await removeWorktree(
          worktreeId,
          {
            mergeTo: removeOptions.mergeTo || undefined,
            deleteBranch: removeOptions.deleteBranch,
            forceBranch: removeOptions.forceBranch,
          },
          async () => {
            await homeViewRef.value?.fadeOutCard(worktreeId);
            savedPositions = homeViewRef.value?.hideCard(worktreeId);
          },
        );
        await nextTick();
        if (savedPositions) homeViewRef.value?.animateAfterRemove(savedPositions);
      } catch (e) {
        if (savedPositions) homeViewRef.value?.animateAfterRemove(savedPositions);
        // git 操作失敗時: 事前処理の副作用をロールバック
        if (onRemoveFailed) {
          try { await onRemoveFailed(worktree); } catch { /* ロールバック失敗は無視 */ }
        }
        await message(t("deleteFailed", { error: e }), { kind: "error" });
      } finally {
        homeViewRef.value?.unhideCard(worktreeId);
      }
    } finally {
      loadingWorktrees.delete(worktreeId);
    }
  }

  async function onRemoveWorktreeConfirm(removeOptions: RemoveOptions): Promise<void> {
    await _confirm(removeOptions, t("deletingText"));
  }

  async function onArchiveWorktreeConfirm(removeOptions: RemoveOptions): Promise<void> {
    await _confirm(
      removeOptions,
      t("archivingText"),
      async (worktree) => {
        await saveArchive({
          id: worktree.id,
          name: worktree.name,
          repository_id: worktree.repositoryId,
          repository_name: worktree.repositoryName,
          path: worktree.path,
          branch_name: worktree.branchName,
          archived_at: Date.now(),
        });
      },
      async (worktree) => {
        // git 操作失敗時: 保存済みアーカイブをロールバック
        await deleteArchive(worktree.id);
      },
    );
  }

  return {
    showRemoveDialog,
    removeTargetWorktree,
    removeBranches,
    removeDirtyFiles,
    onRemoveWorktree,
    onRemoveWorktreeConfirm,
    onArchiveWorktreeConfirm,
    dismissDialog,
  };
}
