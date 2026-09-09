import { describe, expect, it } from "vitest";
import { computed, effectScope, nextTick, ref } from "vue";
import { useTrayWorktreeNav } from "./useTrayWorktreeNav";
import type { TrayWorktreeData } from "./useTrayPopup";

/**
 * #233: トレイポップアップの再入シナリオの回帰テスト。
 *
 * 守りたいのは「外部イベント `tray-notification-cleared` が、ユーザー操作の await の
 * 隙間に割り込んで `allWorktrees` / `currentIndex` を動かさない」こと。壊れると
 * **ダイアログを開いた対象と別のワークツリーがアーカイブされる**（`deleteBranch` 付きなら
 * ブランチも消える）ため、実害が大きい。
 */

function makeWorktree(id: string): TrayWorktreeData {
  return {
    worktreeId: id,
    worktreeName: id,
    worktreePath: `X:/wt/${id}`,
    isDetached: false,
    layout: null,
    terminals: [],
    branchName: `feature/${id}`,
    repositoryName: "oretachi",
    autoApproval: false,
    aiJudging: false,
    canArchive: true,
  };
}

function deferred() {
  let resolve!: () => void;
  const promise = new Promise<void>((r) => {
    resolve = r;
  });
  return { promise, resolve };
}

/** watch(pre) → flushRemovals → goTo と続く非同期の連鎖が落ち着くまで待つ */
async function settle(): Promise<void> {
  for (let i = 0; i < 20; i++) await nextTick();
}

type Hooks = {
  detachCurrentTerminals?: () => Promise<void>;
  clearLeavingNotification?: (worktreeId: string) => Promise<void>;
  notifyEnteringWorktree?: (worktreeId: string) => Promise<void>;
  showWorktree?: (data: TrayWorktreeData) => Promise<void>;
  requestArchive?: (worktreeId: string, deleteBranch: boolean) => Promise<void>;
};

function setup(count: number) {
  const closing = ref(false);
  const showArchiveConfirm = ref(false);
  const showIdeDialog = ref(false);
  const dialogOpen = computed(() => showArchiveConfirm.value || showIdeDialog.value);

  /** 各 dep の途中で await を挟ませたいテストが差し込む */
  const hooks: Hooks = {};
  const detached: string[] = [];
  const cleared: string[] = [];
  const entered: string[] = [];
  const shown: string[] = [];
  const archiveRequests: { worktreeId: string; deleteBranch: boolean }[] = [];
  const closes: number[] = [];

  const scope = effectScope();
  const nav = scope.run(() =>
    useTrayWorktreeNav({
      closing,
      dialogOpen,
      async detachCurrentTerminals() {
        detached.push(nav.currentWorktree.value?.worktreeId ?? "(none)");
        await hooks.detachCurrentTerminals?.();
      },
      async clearLeavingNotification(worktreeId) {
        cleared.push(worktreeId);
        await hooks.clearLeavingNotification?.(worktreeId);
      },
      async notifyEnteringWorktree(worktreeId) {
        entered.push(worktreeId);
        await hooks.notifyEnteringWorktree?.(worktreeId);
      },
      async showWorktree(data) {
        shown.push(data.worktreeId);
        await hooks.showWorktree?.(data);
      },
      async requestArchive(worktreeId, deleteBranch) {
        archiveRequests.push({ worktreeId, deleteBranch });
        await hooks.requestArchive?.(worktreeId, deleteBranch);
      },
      async closePopupWithoutClearing() {
        closes.push(1);
        // 実物の closePopup も closing を立ててから destroy する
        closing.value = true;
      },
    }),
  )!;

  nav.allWorktrees.value = Array.from({ length: count }, (_, i) => makeWorktree(`w${i}`));
  nav.currentIndex.value = 0;

  return {
    nav,
    closing,
    showArchiveConfirm,
    showIdeDialog,
    hooks,
    detached,
    cleared,
    entered,
    shown,
    archiveRequests,
    closes,
    scope,
    ids: () => nav.allWorktrees.value.map((w) => w.worktreeId),
  };
}

describe("useTrayWorktreeNav", () => {
  describe("アーカイブ確認ダイアログ表示中の割り込み", () => {
    it("表示中カードのクリアが来ても、確定される対象は開いた時のワークツリーのまま", async () => {
      const h = setup(3);
      h.showArchiveConfirm.value = true;
      await settle();
      expect(h.nav.currentWorktree.value?.worktreeId).toBe("w0");

      // 開いている間に、表示中カード自身の通知が外でクリアされる
      h.nav.removeWorktreeFromList("w0");
      await settle();

      // 予約は溜まるだけ。一覧を動かすと currentWorktree が w1 に化ける
      expect(h.ids()).toEqual(["w0", "w1", "w2"]);
      expect(h.nav.pendingRemovalCount()).toBe(1);

      // 確定（実物の onArchiveConfirmed と同じ順序）
      h.showArchiveConfirm.value = false;
      await h.nav.archiveCurrent({ deleteBranch: true });

      expect(h.archiveRequests).toEqual([{ worktreeId: "w0", deleteBranch: true }]);
    });

    it("他カードのクリアが来ても一覧は動かず、ダイアログを閉じたら流れる", async () => {
      const h = setup(3);
      h.showArchiveConfirm.value = true;
      await settle();

      h.nav.removeWorktreeFromList("w2");
      await settle();
      expect(h.ids()).toEqual(["w0", "w1", "w2"]);

      h.showArchiveConfirm.value = false;
      await settle();
      expect(h.ids()).toEqual(["w0", "w1"]);
      expect(h.nav.pendingRemovalCount()).toBe(0);
    });

    it("IDE ダイアログでも同じく見送られ、閉じたら watch 経由で流れる", async () => {
      const h = setup(3);
      h.showIdeDialog.value = true;
      await settle();

      h.nav.removeWorktreeFromList("w1");
      await settle();
      expect(h.ids()).toEqual(["w0", "w1", "w2"]);
      expect(h.nav.pendingRemovalCount()).toBe(1);

      h.showIdeDialog.value = false;
      await settle();
      expect(h.ids()).toEqual(["w0", "w2"]);
      expect(h.nav.pendingRemovalCount()).toBe(0);
      // 表示中カードは動いていないので再表示も走らない
      expect(h.shown).toEqual([]);
      expect(h.nav.currentWorktree.value?.worktreeId).toBe("w0");
    });
  });

  describe("goTo の await 中の割り込み", () => {
    it("遷移中はクリアを見送るので、遷移先を1つ飛ばさない", async () => {
      const h = setup(3);
      const gate = deferred();
      h.hooks.clearLeavingNotification = () => gate.promise;

      const p = h.nav.goTo(1); // w0 → w1
      await nextTick();

      // await 中に「先頭 w0 が外でクリアされた」が届く。即 splice すると
      // index 1 が w2 を指し、w1 を飛ばして表示してしまう
      h.nav.removeWorktreeFromList("w0");
      await settle();
      expect(h.ids()).toEqual(["w0", "w1", "w2"]);

      gate.resolve();
      await p;
      expect(h.shown).toEqual(["w1"]);

      // 遷移が終われば予約が流れる（w0 は非表示側なので同期的に抜ける）
      await settle();
      expect(h.ids()).toEqual(["w1", "w2"]);
      expect(h.nav.currentIndex.value).toBe(0);
      expect(h.nav.currentWorktree.value?.worktreeId).toBe("w1");
      expect(h.nav.pendingRemovalCount()).toBe(0);
    });

    it("await 中に一覧が縮んでも末尾へ丸めて表示し、例外にしない", async () => {
      const h = setup(3);
      const gate = deferred();
      h.hooks.clearLeavingNotification = () => gate.promise;

      const p = h.nav.goTo(2); // w0 → w2
      await nextTick();

      // アーカイブ導線など、composable の外から末尾が消えるケース
      h.nav.allWorktrees.value.splice(2, 1);
      gate.resolve();
      await expect(p).resolves.toBeUndefined();

      expect(h.nav.currentIndex.value).toBe(1);
      expect(h.shown).toEqual(["w1"]);
      expect(h.nav.navigating.value).toBe(false);
    });

    it("await 中に一覧が空になっても例外を投げず、何も表示しない", async () => {
      const h = setup(3);
      const gate = deferred();
      h.hooks.clearLeavingNotification = () => gate.promise;

      const p = h.nav.goTo(2);
      await nextTick();

      h.nav.allWorktrees.value = [];
      gate.resolve();
      await expect(p).resolves.toBeUndefined();

      expect(h.shown).toEqual([]);
      expect(h.entered).toEqual([]);
      expect(h.nav.navigating.value).toBe(false);
    });

    it("遷移中の goTo は再入しない", async () => {
      const h = setup(3);
      const gate = deferred();
      h.hooks.clearLeavingNotification = () => gate.promise;

      const first = h.nav.goTo(1);
      await nextTick();
      await h.nav.goTo(2); // 弾かれる

      gate.resolve();
      await first;
      expect(h.shown).toEqual(["w1"]);
    });
  });

  describe("アーカイブ依頼中の割り込み", () => {
    it("依頼の await 中にクリアが来ても、依頼先と splice 位置が入れ替わらない", async () => {
      const h = setup(3);
      const gate = deferred();
      h.hooks.requestArchive = () => gate.promise;

      const p = h.nav.archiveCurrent({ deleteBranch: false });
      await nextTick();

      h.nav.removeWorktreeFromList("w1");
      await settle();
      expect(h.ids()).toEqual(["w0", "w1", "w2"]);

      gate.resolve();
      await p;

      expect(h.archiveRequests).toEqual([{ worktreeId: "w0", deleteBranch: false }]);
      // w0 を抜いて同じ index を再表示
      expect(h.ids()).toEqual(["w1", "w2"]);
      expect(h.shown).toEqual(["w1"]);

      // 依頼が終われば w1 の予約が流れ、続けて w2 が表示される
      await settle();
      expect(h.shown).toEqual(["w1", "w2"]);
      expect(h.ids()).toEqual(["w2"]);
      expect(h.nav.currentWorktree.value?.worktreeId).toBe("w2");
      expect(h.nav.pendingRemovalCount()).toBe(0);
    });

    it("表示中が最後の1件ならポップアップを閉じる", async () => {
      const h = setup(1);
      await h.nav.archiveCurrent({ deleteBranch: true });

      expect(h.archiveRequests).toEqual([{ worktreeId: "w0", deleteBranch: true }]);
      expect(h.closes).toHaveLength(1);
      expect(h.shown).toEqual([]);
    });
  });

  describe("flushRemovals", () => {
    it("表示中カードを抜くとき、末尾なら1つ戻って再表示する", async () => {
      const h = setup(3);
      h.nav.currentIndex.value = 2; // 末尾 w2 を表示中

      h.nav.removeWorktreeFromList("w2");
      await settle();

      expect(h.ids()).toEqual(["w0", "w1"]);
      expect(h.nav.currentIndex.value).toBe(1);
      expect(h.shown).toEqual(["w1"]);
      // 離脱側の通知クリアは外で済んでいるので出さない
      expect(h.cleared).toEqual([]);
    });

    it("表示中カードを抜くとき、後続があれば同じ index を再表示する", async () => {
      const h = setup(3);
      h.nav.currentIndex.value = 1; // w1 を表示中

      h.nav.removeWorktreeFromList("w1");
      await settle();

      expect(h.ids()).toEqual(["w0", "w2"]);
      expect(h.nav.currentIndex.value).toBe(1);
      expect(h.shown).toEqual(["w2"]);
    });

    it("非表示カードが表示中より前なら currentIndex を詰める", async () => {
      const h = setup(3);
      h.nav.currentIndex.value = 2;

      h.nav.removeWorktreeFromList("w0");
      await settle();

      expect(h.ids()).toEqual(["w1", "w2"]);
      expect(h.nav.currentIndex.value).toBe(1);
      expect(h.nav.currentWorktree.value?.worktreeId).toBe("w2");
      // 表示中カードは動いていないので再表示しない
      expect(h.shown).toEqual([]);
    });

    it("複数まとめて予約されても、表示中カードは最後に処理する", async () => {
      const h = setup(4);
      h.nav.currentIndex.value = 1; // w1 を表示中

      h.nav.removeWorktreeFromList("w1");
      h.nav.removeWorktreeFromList("w0");
      h.nav.removeWorktreeFromList("w3");
      await settle();

      expect(h.ids()).toEqual(["w2"]);
      expect(h.nav.currentIndex.value).toBe(0);
      expect(h.nav.currentWorktree.value?.worktreeId).toBe("w2");
      expect(h.nav.pendingRemovalCount()).toBe(0);
      // 表示中カードの取り除きは1回だけ（detach → splice → 再表示）
      expect(h.shown).toEqual(["w2"]);
    });

    it("最後の1件が消えたらポップアップを閉じる", async () => {
      const h = setup(1);
      h.nav.removeWorktreeFromList("w0");
      await settle();

      expect(h.ids()).toEqual([]);
      expect(h.closes).toHaveLength(1);
      // クリアは外で済んでいるので改めて出さない
      expect(h.cleared).toEqual([]);
    });

    it("既に一覧に無い予約は捨てる", async () => {
      const h = setup(2);
      h.nav.removeWorktreeFromList("gone");
      await settle();

      expect(h.ids()).toEqual(["w0", "w1"]);
      expect(h.nav.pendingRemovalCount()).toBe(0);
    });

    it("閉鎖中は一覧を触らない", async () => {
      const h = setup(3);
      h.closing.value = true;

      h.nav.removeWorktreeFromList("w1");
      await settle();

      expect(h.ids()).toEqual(["w0", "w1", "w2"]);
      expect(h.nav.pendingRemovalCount()).toBe(1);
    });
  });

  describe("cancelNavigation", () => {
    it("進行中の goTo の続きを打ち切る", async () => {
      const h = setup(3);
      const gate = deferred();
      h.hooks.clearLeavingNotification = () => gate.promise;

      const p = h.nav.goTo(1);
      await nextTick();

      h.nav.cancelNavigation();
      gate.resolve();
      await p;

      expect(h.shown).toEqual([]);
      expect(h.entered).toEqual([]);
    });
  });
});
