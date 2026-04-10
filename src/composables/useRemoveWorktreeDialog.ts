import { ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import type { useWorktreeRemove, RemoveOptions } from "./useWorktreeRemove";

export function useRemoveWorktreeDialog(core: ReturnType<typeof useWorktreeRemove>) {
  const { worktrees, listBranches, clearNotification, cancellableWorktrees, cancelWorktreeRemove } = core;

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

  /** ダイアログ確認後の共通処理: state をクリアしてコア処理に委譲する */
  async function _confirm(
    removeOptions: RemoveOptions,
    execute: (worktreeId: string, removeOptions: RemoveOptions) => Promise<void>,
  ): Promise<void> {
    if (!removeTargetWorktree.value) return;
    const { id: worktreeId } = removeTargetWorktree.value;

    if (!worktrees.value.find((w) => w.id === worktreeId)) {
      showRemoveDialog.value = false;
      removeDirtyFiles.value = [];
      return;
    }

    showRemoveDialog.value = false;
    removeTargetWorktree.value = null;
    removeBranches.value = [];
    removeDirtyFiles.value = [];

    await execute(worktreeId, removeOptions);
  }

  async function onRemoveWorktreeConfirm(removeOptions: RemoveOptions): Promise<void> {
    await _confirm(removeOptions, core.removeWorktreeNoArchive);
  }

  async function onArchiveWorktreeConfirm(removeOptions: RemoveOptions): Promise<void> {
    await _confirm(removeOptions, core.archiveWorktree);
  }

  return {
    showRemoveDialog,
    removeTargetWorktree,
    removeBranches,
    removeDirtyFiles,
    cancellableWorktrees,
    onRemoveWorktree,
    onRemoveWorktreeConfirm,
    onArchiveWorktreeConfirm,
    cancelWorktreeRemove,
    dismissDialog,
  };
}
