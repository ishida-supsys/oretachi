import { reactive, nextTick } from "vue";
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

export type WorktreeRemoveOptions = {
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
};

export type RemoveOptions = { mergeTo: string; deleteBranch: boolean; forceBranch: boolean };

/** 削除処理の最終結果。MCP 経由の呼び出し元へ成否を返すために使う */
export type WorktreeRemoveResult =
  | { ok: true }
  | { ok: false; cancelled: boolean; error: string };

/** 結果が確定した時点で呼ばれるコールバック（エラーダイアログ表示より前に呼ばれる） */
export type OnRemoveSettled = (result: WorktreeRemoveResult) => Promise<void>;

export function useWorktreeRemove(options: WorktreeRemoveOptions) {
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
  }).catch(() => { /* リスナー登録失敗時は無視（通常発生しない） */ });

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

  /** コア削除処理: ターミナル停止・アニメーション付きワークツリー削除
   *  beforeRemove: git 操作前に呼ぶ任意の非同期処理（アーカイブ保存など）
   *  onRemoveFailed: git 操作失敗時に beforeRemove の副作用をロールバックするコールバック
   *  afterRemove: git 操作成功後に呼ぶ任意の非同期処理（MCP通知など）
   *  onSettled: 成否が確定した時点（エラーダイアログ表示より前）に呼ぶ任意の非同期処理
   */
  async function _execute(
    worktreeId: string,
    removeOptions: RemoveOptions,
    loadingText: string,
    beforeRemove?: (worktree: Worktree) => Promise<void>,
    onRemoveFailed?: (worktree: Worktree) => Promise<void>,
    afterRemove?: (worktree: Worktree) => Promise<void>,
    onSettled?: OnRemoveSettled,
  ): Promise<WorktreeRemoveResult> {
    // 結果は必ず一度だけ確定・通知する（エラーダイアログは await でブロックするため通知はその前に行う）
    let settledResult: WorktreeRemoveResult | null = null;
    async function settle(result: WorktreeRemoveResult): Promise<WorktreeRemoveResult> {
      if (settledResult) return settledResult;
      settledResult = result;
      if (onSettled) {
        try { await onSettled(result); } catch { /* 通知失敗は削除の成否に影響しない */ }
      }
      return result;
    }
    const failure = (e: unknown): WorktreeRemoveResult => ({
      ok: false,
      cancelled: String(e) === "cancelled",
      error: String(e),
    });

    const worktree = worktrees.value.find((w) => w.id === worktreeId);
    if (!worktree) {
      return await settle({ ok: false, cancelled: false, error: `worktree ${worktreeId} not found` });
    }

    clearNotification(worktreeId);

    // UI 破壊的操作の前に事前処理（アーカイブ保存など）を実行する。
    // ここで失敗した場合はワークツリーに何も手を加えずエラーを表示して終了する。
    if (beforeRemove) {
      try {
        await beforeRemove(worktree);
      } catch (e) {
        const result = await settle(failure(e));
        await message(t("deleteFailed", { error: e }), { kind: "error" });
        return result;
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

      // ターミナルプロセスをkillする（ディレクトリのファイルハンドル解放が目的）。
      // ただし UI 状態のクリア（terminals.splice / frameBundles.delete）は削除成功後に行う。
      // キャンセル時にワークツリーが UI に残った際、空ターミナルではなく停止済み状態で表示できるようにする。
      const bundle = worktreeFrameBundles.get(worktreeId);
      const terminalsSnapshot = [...worktree.terminals];
      for (const terminal of terminalsSnapshot) {
        const term = bundle?.terminalRefs.get(terminal.id) ?? getTerminalRef(terminal.id);
        if (term?.isRunning) await term.kill();
      }

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
        // 削除成功後に UI 状態をクリア
        for (const terminal of terminalsSnapshot) {
          terminalWorktreeMap.delete(terminal.id);
        }
        worktree.terminals.splice(0);
        worktreeFrameBundles.delete(worktreeId);
        // git 操作成功後の後処理（MCP通知など）
        if (afterRemove) {
          try { await afterRemove(worktree); } catch { /* 通知失敗はワークツリー削除の成否に影響しない */ }
        }
        await settle({ ok: true });
        await nextTick();
        if (savedPositions) homeViewRef.value?.animateAfterRemove(savedPositions);
      } catch (e) {
        if (savedPositions) homeViewRef.value?.animateAfterRemove(savedPositions);
        // git 操作失敗時: 事前処理の副作用をロールバック
        if (onRemoveFailed) {
          try { await onRemoveFailed(worktree); } catch { /* ロールバック失敗は無視 */ }
        }
        await settle(failure(e));
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
      await settle(failure(e));
      if (String(e) !== "cancelled") {
        await message(t("deleteFailed", { error: e }), { kind: "error" });
      }
    } finally {
      loadingWorktrees.delete(worktreeId);
      cancellableWorktrees.delete(worktreeId);
    }

    // ここに到達して結果が未確定なのは想定外（通常発生しない）。安全側に倒して不明=失敗として返す
    return settledResult ?? (await settle({ ok: false, cancelled: false, error: "unknown" }));
  }

  /** アーカイブDBに保存してワークツリーを削除する（ダイアログ経由・MCP経由の共通処理） */
  async function archiveWorktree(
    worktreeId: string,
    removeOptions: RemoveOptions,
    onSettled?: OnRemoveSettled,
  ): Promise<WorktreeRemoveResult> {
    return await _execute(
      worktreeId,
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
          description: worktree.description ?? null,
          workgroup_id: worktree.workgroupId ?? null,
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
      onSettled,
    );
  }

  /** アーカイブDBへの保存なしでワークツリーを削除する */
  async function removeWorktreeNoArchive(
    worktreeId: string,
    removeOptions: RemoveOptions,
    onSettled?: OnRemoveSettled,
  ): Promise<WorktreeRemoveResult> {
    return await _execute(worktreeId, removeOptions, t("deletingText"), undefined, undefined, undefined, onSettled);
  }

  return {
    worktrees,
    listBranches,
    clearNotification,
    cancellableWorktrees,
    cancelWorktreeRemove,
    archiveWorktree,
    removeWorktreeNoArchive,
  };
}
