import { ref, nextTick } from "vue";
import type { Ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { message } from "@tauri-apps/plugin-dialog";
import { useWorktrees } from "./useWorktrees";
import { cancelApproval } from "../utils/autoApproval";
import { saveArchive } from "./useArchivePersistence";
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
   *  afterRemove: git 操作成功後に呼ぶ任意の非同期処理（アーカイブ保存など）
   */
  async function _confirm(
    removeOptions: RemoveOptions,
    loadingText: string,
    afterRemove?: (worktree: Worktree) => Promise<void>,
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
        // git 操作成功後にアーカイブ保存などを実行
        if (afterRemove) await afterRemove(worktree);
        await nextTick();
        if (savedPositions) homeViewRef.value?.animateAfterRemove(savedPositions);
      } catch (e) {
        if (savedPositions) homeViewRef.value?.animateAfterRemove(savedPositions);
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
    await _confirm(removeOptions, t("archivingText"), async (worktree) => {
      await saveArchive({
        id: worktree.id,
        name: worktree.name,
        repository_id: worktree.repositoryId,
        repository_name: worktree.repositoryName,
        path: worktree.path,
        branch_name: worktree.branchName,
        archived_at: Date.now(),
      });
    });
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
