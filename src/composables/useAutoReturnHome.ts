import { watch, type Ref } from "vue";
import type { AppSettings } from "../types/settings";

/** タスク完了後、メインウィンドウが非フォーカスのまま経過したらホームへ戻す待ち時間 */
export const AUTO_RETURN_HOME_DELAY_MS = 5000;

export interface AutoReturnHomeDeps {
  /** ワークグループ設定と worktree の所属を引くための設定 */
  settings: Ref<AppSettings>;
  /** worktree の実効グループID（未設定/不明なら先頭グループ。useWorkgroups.resolvedGroupId） */
  resolvedGroupId: (workgroupId: string | undefined) => string;
  /** メインウィンドウのフォーカス状態 */
  isWindowFocused: Ref<boolean>;
  /** サブウィンドウへ移されたワークツリーか（メインのタブが動かないので対象外にする） */
  isDetached: (worktreeId: string) => boolean;
  /** ホームタブへ戻す */
  goHome: () => void;
}

/**
 * タスク完了後にホームタブへ自動復帰する予約を管理する。
 *
 * 予約の意味は「メインウィンドウが非フォーカスのまま {@link AUTO_RETURN_HOME_DELAY_MS} 経過したら
 * ホームへ戻す」。完了時点でフォーカスしていても予約は張り、フォーカスが外れた時点で
 * カウントダウンを始める。フォーカスが戻ったらカウントダウンだけ止め、次に外れたら再開する。
 * （旧実装は「完了時点でフォーカス中なら予約しない」だったため、タスク完了を見届けてから
 *   離席したケースで永久に復帰しなかった。#224）
 *
 * 予約が消えるのは次のタスクが走り出したとき / 復帰が実行されたとき / アンマウント時のみ。
 */
export function useAutoReturnHome(deps: AutoReturnHomeDeps) {
  let timer: ReturnType<typeof setTimeout> | null = null;
  let unwatch: (() => void) | null = null;

  function clearCountdown(): void {
    if (timer !== null) {
      clearTimeout(timer);
      timer = null;
    }
  }

  /** 予約そのものを破棄する（フォーカス監視も外す） */
  function cancel(): void {
    clearCountdown();
    if (unwatch) {
      unwatch();
      unwatch = null;
    }
  }

  function startCountdown(): void {
    clearCountdown();
    timer = setTimeout(() => {
      timer = null;
      cancel();
      deps.goHome();
    }, AUTO_RETURN_HOME_DELAY_MS);
  }

  /**
   * 復帰予約を入れる。
   *
   * @param createdWorktreeId タスクが生成したワークツリーID。生成なし（既存ワークツリーへの
   *   agent_worktree のみ）ならタブが動かないので予約しない。
   * @param fallbackGroupId 生成されたワークツリーが settings に見つからなかったときに使う
   *   グループID（タスク追加時点で確定させたもの）。
   */
  function schedule(createdWorktreeId: string | null, fallbackGroupId?: string): void {
    if (!createdWorktreeId) return;
    if (deps.isDetached(createdWorktreeId)) return;
    // 実際の所属は executeAddWorktree が実行時の activeWorkgroupId で決めるため、
    // 追加時点で見込んだグループとは食い違いうる。生成されたエントリの所属を正とする。
    const entry = deps.settings.value.worktrees?.find((w) => w.id === createdWorktreeId);
    const groupId = deps.resolvedGroupId(entry?.workgroupId ?? fallbackGroupId);
    const group = deps.settings.value.workgroups?.find((g) => g.id === groupId);
    if (!group?.autoReturnHomeAfterTask) return;

    cancel();
    unwatch = watch(deps.isWindowFocused, (focused) => {
      // 見に来ている間は待機し、離れたらカウントダウンを開始/再開する
      if (focused) clearCountdown();
      else startCountdown();
    });
    if (!deps.isWindowFocused.value) startCountdown();
  }

  return { schedule, cancel };
}
