import { ref, reactive, nextTick } from "vue";
import type { Ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
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

  // 永続リトライ中のワークツリーID（キャンセルボタン表示用）
  const cancellableWorktrees = reactive(new Set<string>());

  // バックエンドから永続リトライ開始イベントを受信したらキャンセル可能状態に移行
  listen<{ worktreePath: string }>("worktree-remove-retrying", (event) => {
    const worktree = worktrees.value.find(
      (w) => w.path === event.payload.worktreePath
    );
    if (worktree) {
      cancellableWorktrees.add(worktree.id);
      loadingWorktrees.set(worktree.id, t("retryingDeleteText"));
    }
  });

  /** 永続リトライ中の削除をキャンセルする */
  async function cancelWorktreeRemove(worktreeId: string): Promise<void> {
    const worktree = worktrees.value.find((w) => w.id === worktreeId);
    if (!worktree) return;
    try {
      await invoke("cancel_worktree_remove", { worktreePath: worktree.path });
    } catch {
      // すでに完了している場合は無視
    }
  }

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
   *  afterRemove: git 操作成功後に呼ぶ任意の非同期処理（MCP通知など）
   */
  async function _confirm(
    removeOptions: RemoveOptions,
    loadingText: string,
    beforeRemove?: (worktree: Worktree) => Promise<void>,
    onRemoveFailed?: (worktree: Worktree) => Promise<void>,
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

    // ダイアログを閉じた後、UI 破壊的操作の前に事前処理（アーカイブ保存など）を実行する。
    // ここで失敗した場合はワークツリーに何も手を加えずエラーを表示して終了する。
    if (beforeRemove) {
      try {
        await beforeRemove(worktree);
      } catch (e) {
        await message(t("deleteFailed", { error: e }), { kind: "error" });
        return;
      }
    }

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
      worktree.terminals.splice(0);
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
        // git 操作成功後の後処理（MCP通知など）
        if (afterRemove) {
          try { await afterRemove(worktree); } catch { /* 通知失敗はワークツリー削除の成否に影響しない */ }
        }
        await nextTick();
        if (savedPositions) homeViewRef.value?.animateAfterRemove(savedPositions);
      } catch (e) {
        if (savedPositions) homeViewRef.value?.animateAfterRemove(savedPositions);
        // git 操作失敗時: 事前処理の副作用をロールバック
        if (onRemoveFailed) {
          try { await onRemoveFailed(worktree); } catch { /* ロールバック失敗は無視 */ }
        }
        // "cancelled" はユーザー操作によるキャンセルなのでエラーダイアログを出さない
        if (String(e) !== "cancelled") {
          await message(t("deleteFailed", { error: e }), { kind: "error" });
        }
      } finally {
        homeViewRef.value?.unhideCard(worktreeId);
        cancellableWorktrees.delete(worktreeId);
      }
    } catch (e) {
      // UI 後処理ステップ（moveToMainWindow・closeArtifactWindow 等）が失敗した場合。
      // beforeRemove が実行済みであれば副作用をロールバックする。
      if (onRemoveFailed) {
        try { await onRemoveFailed(worktree); } catch { /* ロールバック失敗は無視 */ }
      }
      if (String(e) !== "cancelled") {
        await message(t("deleteFailed", { error: e }), { kind: "error" });
      }
    } finally {
      loadingWorktrees.delete(worktreeId);
      cancellableWorktrees.delete(worktreeId);
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
        // git 操作失敗時: ワークツリーパスがまだ存在する場合のみアーカイブをロールバック
        // (git_worktree_remove 成功後に git_delete_branch が失敗した場合はワークツリーは
        //  既に消えているためアーカイブを保持する)
        const pathStillExists = await invoke<boolean>("path_exists", { path: worktree.path }).catch(() => false);
        if (pathStillExists) {
          await deleteArchive(worktree.id);
        } else {
          // ワークツリーは削除済み（ブランチ削除失敗などの部分的失敗）→ afterRemove は呼ばれないため
          // ここでMCPクライアントへ通知する
          await invoke("notify_worktree_archived", {
            id: worktree.id,
            name: worktree.name,
            branchName: worktree.branchName,
          }).catch(() => { /* 通知失敗は無視 */ });
        }
      },
      async (worktree) => {
        // git 操作成功後: MCPクライアントへアーカイブ完了を通知
        await invoke("notify_worktree_archived", {
          id: worktree.id,
          name: worktree.name,
          branchName: worktree.branchName,
        });
      },
    );
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
