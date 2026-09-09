import { computed, ref, watch, type ComputedRef, type Ref } from "vue";
import type { TrayWorktreeData } from "./useTrayPopup";

/**
 * トレイポップアップの「一覧 + 現在位置」の変異を直列化する（#218 / #233）。
 *
 * `allWorktrees` / `currentIndex` には変異経路が 2 つある:
 *   1. ユーザー操作（前へ / 次へ / アーカイブ）
 *   2. 外部イベント `tray-notification-cleared`（通知レポートから返答した宛先など）
 *
 * 2 は 1 の await の隙間に割り込むため、素直に splice すると
 *   - 確認ダイアログを開いた対象と**別のワークツリーがアーカイブされる**
 *   - `goTo` が await 後に読む index がずれ、1 つ飛ばす / `entering` が undefined で例外
 * が起きる。そこで 2 は `pendingRemovals` へ予約し、「遷移中でもダイアログ中でも
 * アーカイブ依頼中でもない」瞬間にだけ流す。
 *
 * SFC から切り出してあるのは、この直列化が壊れたことを自動テストで検知するため。
 */
export type TrayWorktreeNavDeps = {
  /** 閉鎖処理中フラグ（closePopup 側が持つ）。立っている間は一切変異させない */
  closing: Ref<boolean>;
  /**
   * 表示中カードを差し替えてはいけない状態（確認 / IDE / 自動承認ダイアログ）。
   * ダイアログは `currentWorktree` にリアクティブ束縛されているため。
   */
  dialogOpen: Ref<boolean> | ComputedRef<boolean>;
  /** トレイが掴んでいるターミナルを手放す */
  detachCurrentTerminals: () => Promise<void>;
  /** 離脱側の通知を既読にする */
  clearLeavingNotification: (worktreeId: string) => Promise<void>;
  /** 表示先が変わったことを main へ伝える */
  notifyEnteringWorktree: (worktreeId: string) => Promise<void>;
  /** 入る側を実際に描画する */
  showWorktree: (data: TrayWorktreeData) => Promise<void>;
  /** main へアーカイブを依頼する */
  requestArchive: (worktreeId: string, deleteBranch: boolean) => Promise<void>;
  /**
   * ポップアップを閉じる。通知クリアは外（main の archiveWorktree / 通知クリア元）で
   * 済んでいるため、改めて出さない経路だけを使う
   */
  closePopupWithoutClearing: () => Promise<void>;
};

export type GoToOptions = {
  /**
   * 離れる側の通知を既読にする（既定 true）。アーカイブ導線では main 側の
   * archiveWorktree が内部で clearNotification するため false にする。
   */
  clearLeaving?: boolean;
  /** 呼び出し側で detachCurrentTerminals() を済ませている場合 true */
  alreadyDetached?: boolean;
};

export function useTrayWorktreeNav(deps: TrayWorktreeNavDeps) {
  /** 全ワークツリーデータ（開いた時点のスナップショット） */
  const allWorktrees = ref<TrayWorktreeData[]>([]);
  const currentIndex = ref(0);
  /** 遷移中フラグ（連打・ホットキー二重発火の抑止 + ボタンの disabled 用） */
  const navigating = ref(false);
  /**
   * アーカイブ依頼の進行中フラグ。`archiveCurrent` は await を挟んでから一覧を
   * splice するので、その間に外からの通知クリア（#218）で一覧を触られてはいけない
   */
  const archiving = ref(false);
  /**
   * 遷移の世代。await の後で自分が最新でなければ以降の副作用を捨てる
   * （destroy 済みウィンドウへの setSize や、打ち切った遷移の描画を防ぐ）
   */
  let navToken = 0;

  const currentWorktree = computed(() => allWorktrees.value[currentIndex.value] ?? null);
  const isLast = computed(() => currentIndex.value >= allWorktrees.value.length - 1);
  const isFirst = computed(() => currentIndex.value <= 0);

  /**
   * 外からクリアされたが、まだ一覧から外せていないワークツリー（#218）。
   * **捨てずに溜めて後で流す**（捨てると「捌き終わったカードが巡回に残る」に戻る）。
   */
  const pendingRemovals = new Set<string>();
  /** `flushRemovals` の再入防止。goTo / closePopup を await する間に watch から再入する */
  let flushing = false;

  /** 進行中の goTo の続きを打ち切る（closePopup から呼ぶ） */
  function cancelNavigation(): void {
    navToken++;
  }

  /**
   * index のワークツリーを表示する。前へ / 次へ / アーカイブ後の再表示で共有する。
   * - 離れる側の副作用: detach（+ 任意で通知クリア）
   * - 入る側の副作用: tray-current-worktree-changed → showWorktree
   * アーカイブ後は splice 済みで index が据え置きになるため、同一 index への goTo も
   * 再表示として成立させる（同一 index を弾かない）。
   */
  async function goTo(index: number, options: GoToOptions = {}): Promise<void> {
    const { clearLeaving = true, alreadyDetached = false } = options;
    if (deps.closing.value || navigating.value) return;
    if (index < 0 || index >= allWorktrees.value.length) return;

    const token = ++navToken;
    navigating.value = true;
    try {
      const leaving = currentWorktree.value;
      if (!alreadyDetached) await deps.detachCurrentTerminals();
      if (clearLeaving && leaving) {
        await deps.clearLeavingNotification(leaving.worktreeId);
      }
      // await 中に閉鎖 / 別遷移が始まっていたら以降は行わない
      if (token !== navToken || deps.closing.value) return;

      // await を挟む間に一覧が縮んでいることがある（外からの通知クリアの取り込み /
      // アーカイブ）。範囲外のまま進むと `entering` が undefined になり、
      // `entering.worktreeId` で例外を投げて**ターミナルを手放したまま何も表示されない**
      // 状態で固まる。末尾側へ丸めて必ず何か表示する
      const target = Math.min(index, allWorktrees.value.length - 1);
      if (target < 0) return;
      currentIndex.value = target;
      const entering = allWorktrees.value[target];
      await deps.notifyEnteringWorktree(entering.worktreeId);
      if (token !== navToken || deps.closing.value) return;
      await deps.showWorktree(entering);
    } finally {
      if (token === navToken) navigating.value = false;
    }
  }

  async function goNext(): Promise<void> {
    if (!isLast.value) await goTo(currentIndex.value + 1);
  }

  async function goPrev(): Promise<void> {
    // 「見終わった」の意味づけは方向に依らないので、前へでも離脱側は既読にする（冪等）
    if (!isFirst.value) await goTo(currentIndex.value - 1);
  }

  /**
   * 表示中ワークツリーのアーカイブを main へ依頼し、トレイ側は先に進む。
   *
   * 確認ダイアログを閉じた直後に呼ばれる。`archiving` を立てている間は
   * `flushRemovals` が一覧を触らないため、依頼先と splice 位置が入れ替わらない。
   */
  async function archiveCurrent(options: { deleteBranch: boolean }): Promise<void> {
    const wt = currentWorktree.value;
    if (!wt || deps.closing.value) return;

    archiving.value = true;
    try {
      // main 側のターミナル kill / git worktree remove とトレイの attach が競合しないよう、
      // アーカイブ依頼より前にトレイ側のターミナルを必ず切り離す
      await deps.detachCurrentTerminals();

      // アーカイブは数秒〜数十秒かかり、失敗時のエラーダイアログは main 側に出る。
      // トレイは完了を待たずに次へ進む。通知クリアも archiveWorktree 内の clearNotification
      // が行うので tray-clear-notification は出さない。
      await deps.requestArchive(wt.worktreeId, options.deleteBranch);

      // index は捕まえ直す（`archiving` で守ってはいるが、位置の根拠を
      // 「await 前の currentIndex」ではなく worktreeId 一致に寄せておく）
      const index = allWorktrees.value.findIndex((w) => w.worktreeId === wt.worktreeId);
      const wasLast = index < 0 || index >= allWorktrees.value.length - 1;

      if (wasLast) {
        // 「アーカイブ化して完了」: 表示中が最後の1件 → ポップアップを閉じる
        await deps.closePopupWithoutClearing();
        return;
      }

      // 「アーカイブ化して次へ」: 一覧から取り除くと後続が詰まるので、同じ index を再表示する。
      // wasLast === false なので splice 後も index <= length - 1 が保証される。
      allWorktrees.value.splice(index, 1);
      await goTo(index, { clearLeaving: false, alreadyDetached: true });
    } finally {
      archiving.value = false;
    }
  }

  /**
   * 表示中の一覧から、クリア済みのワークツリーを取り除く（#218）。
   *
   * トレイの一覧は開いた時点のスナップショットなので、開いている間に外から通知が
   * クリアされると（通知レポートから返答を送った宛先など）捌き終わったカードが
   * 巡回に残り続ける。離脱側の通知クリアは既に外で済んでいるため `clearLeaving` は立てない。
   *
   * 非表示のカードを先に全部抜いてから表示中カードを処理する。順序を逆にすると、
   * 表示中カードの `goTo` の await 中に残りを抜くことになり、`goTo` が読む index がずれる。
   */
  async function flushRemovals(): Promise<void> {
    if (flushing) return;
    if (deps.closing.value || navigating.value || archiving.value || deps.dialogOpen.value) return;
    if (pendingRemovals.size === 0) return;

    flushing = true;
    try {
      // 非表示のカードは表示中カードを動かさずに抜けるので同期的に片付ける
      for (const worktreeId of Array.from(pendingRemovals)) {
        const index = allWorktrees.value.findIndex((w) => w.worktreeId === worktreeId);
        if (index < 0) {
          // 既に一覧に無い（アーカイブ導線が抜いた等）。溜めておく意味が無い
          pendingRemovals.delete(worktreeId);
          continue;
        }
        if (index === currentIndex.value) continue; // 表示中は後回し
        allWorktrees.value.splice(index, 1);
        if (index < currentIndex.value) currentIndex.value -= 1;
        pendingRemovals.delete(worktreeId);
      }

      const current = currentWorktree.value;
      if (!current || !pendingRemovals.has(current.worktreeId)) return;
      pendingRemovals.delete(current.worktreeId);

      // 表示中のカードが消える。トレイが掴んでいるターミナルを先に手放す
      const index = currentIndex.value;
      await deps.detachCurrentTerminals();
      allWorktrees.value.splice(index, 1);
      if (allWorktrees.value.length === 0) {
        // 最後の1件だった。クリアは外で済んでいるので改めて出さない
        await deps.closePopupWithoutClearing();
        return;
      }
      // splice で後続が詰まるので、末尾を消したときだけ1つ戻る
      await goTo(Math.min(index, allWorktrees.value.length - 1), {
        clearLeaving: false,
        alreadyDetached: true,
      });
    } finally {
      flushing = false;
    }
    // 表示中カードの処理中に溜まった分を続けて流す。1 周ごとに必ず 1 件以上
    // `pendingRemovals` から落ちるので止まる
    if (pendingRemovals.size > 0) void flushRemovals();
  }

  /** 外からクリアされたワークツリーを取り除き予約へ積む（実際に抜くのは `flushRemovals`） */
  function removeWorktreeFromList(worktreeId: string): void {
    pendingRemovals.add(worktreeId);
    void flushRemovals();
  }

  // 取り除きを見送った条件が解けたら流し直す。見送りっぱなしにするとカードが残る
  watch([navigating, archiving, deps.dialogOpen], () => {
    if (pendingRemovals.size > 0) void flushRemovals();
  });

  return {
    allWorktrees,
    currentIndex,
    navigating,
    archiving,
    currentWorktree,
    isFirst,
    isLast,
    cancelNavigation,
    goTo,
    goNext,
    goPrev,
    archiveCurrent,
    flushRemovals,
    removeWorktreeFromList,
    /** 未処理の取り除き予約の件数（テスト / デバッグ用） */
    pendingRemovalCount: () => pendingRemovals.size,
  };
}
