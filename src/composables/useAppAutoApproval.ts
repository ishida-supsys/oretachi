import { reactive } from "vue";
import { listen, emitTo } from "@tauri-apps/api/event";
import { logDebug } from "../utils/log";
import { runApprovalLoop, cancelApproval } from "../utils/autoApproval";
import type { TerminalForApproval } from "../utils/autoApproval";
import type { Ref } from "vue";
import type { Worktree } from "../types/worktree";
import type { AppSettings } from "../types/settings";
import type { NotifyWorktreeEvent } from "./useNotifications";
import type { NotifyKind } from "../types/settings";
import { resolveKindSetting } from "../utils/notificationKinds";
import {
  createPendingNotifyStore,
  queuePendingNotify,
  shouldNotifyAfterJudge,
  takePendingNotify,
  trayOf,
} from "../utils/autoApprovalNotify";
import type TerminalView from "../components/TerminalView.vue";

interface UseAppAutoApprovalDeps {
  worktrees: Ref<Worktree[]>;
  settings: Ref<AppSettings>;
  scheduleSave: () => void;
  isDetached: (id: string) => boolean;
  getTerminalRef: (id: number) => InstanceType<typeof TerminalView> | undefined;
  autoApprovalPromptMap: Map<string, string>;
  lastJudgedCommandMap: Map<string, string>;
  addNotification: (id: string, kind: NotifyKind) => void;
  isWorktreeFocused: (id: string) => boolean;
  onClickAutoApproval: (id: string) => void;
  playSoundForKind: (kind: NotifyKind) => void;
  sendOsNotification: (name: string, title: string, kind?: NotifyKind) => Promise<void>;
  t: (key: string) => string;
}

export function useAppAutoApproval(deps: UseAppAutoApprovalDeps) {
  const autoApprovalMap = reactive(new Map<string, boolean>());
  const aiJudgingWorktrees = reactive(new Set<string>());
  // AI 判定中に届いた notify-worktree の預かり分（#168）。判定完了後に必ず提示する。
  const pendingNotify = createPendingNotifyStore();

  /**
   * 承認待ちとしてユーザーに提示する（バッジ + 通知音 + OS通知）。
   * バッジの count はイベント件数ぶん加算するが、通知音と OS 通知は 1 回に畳む
   * （判定結果ぶんと預かりぶんが同時に立つと音が重なるため）。
   *
   * **種別ごとの ON/OFF（#140）をここで見る。** 自動承認が ON のワークツリーでは
   * `shouldHold` が true を返して `useNotifications` 側のリスナーが早期 return するため、
   * この関数が approval 通知の**唯一の出口**になる。ここを素通しにすると、設定で
   * approval を OFF にしてもバッジと OS 通知だけが出続ける。
   */
  async function notifyApproval(worktreeId: string, worktreeName: string | undefined, count: number) {
    if (count <= 0) return;
    if (!resolveKindSetting(deps.settings.value.notificationSound, "approval").enabled) return;
    for (let i = 0; i < count; i++) deps.addNotification(worktreeId, "approval");
    deps.playSoundForKind("approval");
    if (worktreeName) {
      // `kind` を渡して OS 通知側のゲートも通す（渡さないと素通りする）
      await deps.sendOsNotification(worktreeName, deps.t("notification.titleApproval"), "approval");
    }
  }

  async function onToggleAutoApproval(worktreeId: string) {
    const current = autoApprovalMap.get(worktreeId) ?? false;
    autoApprovalMap.set(worktreeId, !current);

    const wtEntry = deps.settings.value.worktrees.find((w) => w.id === worktreeId);
    if (wtEntry) {
      wtEntry.autoApproval = !current;
      deps.scheduleSave();
    }

    if (current && aiJudgingWorktrees.has(worktreeId)) {
      await cancelApproval(worktreeId);
      if (deps.isDetached(worktreeId)) {
        await emitTo(`sub-${worktreeId}`, "sub-cancel-auto-approve", {});
      }
    }

    if (deps.isDetached(worktreeId)) {
      await emitTo(`sub-${worktreeId}`, "sub-set-auto-approval", { autoApproval: !current });
    }
  }

  async function onCancelAiJudging(worktreeId: string) {
    await cancelApproval(worktreeId);
    if (deps.isDetached(worktreeId)) {
      await emitTo(`sub-${worktreeId}`, "sub-cancel-auto-approve", {});
    }
  }

  /**
   * 自動承認 ON のワークツリーに届いた notify-worktree を処理する。
   * @param tray このイベントをトレイ通知として出してよいか（`tray !== false`）
   */
  async function handleNotify(wt: Worktree, tray: boolean) {
    logDebug(
      `[AutoApproval] notify-worktree received worktreeName=${wt.name} resolved=${wt.id} autoApproval=true tray=${tray}`
    );

    if (aiJudgingWorktrees.has(wt.id)) {
      // ここで捨てると明示 notify_worktree（自動承認 ON では shouldHold で保留されるため
      // この経路しか出口が無い）が黙って消える。判定完了後に提示する（#168）。
      // 預かり分は判定を回さないので、承認済みプロンプト由来のフック通知が
      // 紛れ込むと余分な通知になりうるが、握り潰すより優先している。
      queuePendingNotify(pendingNotify, wt.id, tray);
      logDebug(`[AutoApproval] already in progress for ${wt.id}, queued for later (tray=${tray})`);
      return;
    }

    if (deps.isDetached(wt.id)) {
      logDebug(`[AutoApproval] delegating to sub-window ${wt.id}`);
      // tray はサブウィンドウ経由で sub-auto-approve-result に載って戻ってくる
      await emitTo(`sub-${wt.id}`, "sub-try-auto-approve", {
        additionalPrompt: deps.autoApprovalPromptMap.get(wt.id) ?? "",
        tray,
      });
      return;
    }

    logDebug(`[AutoApproval] local terminals check, count=${wt.terminals.length}`);
    aiJudgingWorktrees.add(wt.id);
    let loopResult: { approved: boolean; lastCommand: string | undefined };
    try {
      const terminalForApproval: TerminalForApproval[] = wt.terminals.flatMap((t) => {
        const ref = deps.getTerminalRef(t.id);
        if (!ref) return [];
        // **承認の Enter は `writeLocked` で送る（#215）。** `oretachi_answer_prompt` が
        // 「fingerprint を照合 → 矢印で ❯ を動かす → CR で確定」を行っている区間に
        // 素の write で CR が割り込むと、移動途中の ❯ が指す選択肢（許可ダイアログなら
        // `2. Yes, and don't ask again` = 以後の無条件承認）を確定させてしまう
        return [{ id: t.id, getTerminal: () => ref.getTerminal(), write: (d: string) => ref.writeLocked(d) }];
      });
      loopResult = await runApprovalLoop(
        terminalForApproval,
        wt.id,
        wt.path,
        deps.autoApprovalPromptMap.get(wt.id),
      );
    } finally {
      aiJudgingWorktrees.delete(wt.id);
    }
    if (loopResult.lastCommand) {
      deps.lastJudgedCommandMap.set(wt.id, loopResult.lastCommand);
    }
    // 預かり分は通知より先に取り出す。後続が throw してもストアに残さない
    // （残すと次回の判定完了時に古いイベント由来の通知が余分に出る）。
    const pending = takePendingNotify(pendingNotify, wt.id);
    const focused = deps.isWorktreeFocused(wt.id);
    let count = 0;
    if (shouldNotifyAfterJudge({ approved: loopResult.approved, focused, tray })) {
      logDebug(`[AutoApproval] local: not approved → addNotification(${wt.id})`);
      count++;
    }
    // 預かり分は判定を回していないので approved 扱いにはしない
    if (pending && shouldNotifyAfterJudge({ approved: false, focused, tray: pending.tray })) {
      logDebug(`[AutoApproval] flush queued notification for ${wt.id}`);
      count++;
    }
    await notifyApproval(wt.id, wt.name, count);
  }

  /**
   * 保存された自動承認状態を `autoApprovalMap` へ復元する。
   *
   * **`init()` とは別に呼べるようにしてある（#256）。** サブウィンドウの復元は
   * `moveToSubWindow` に `autoApprovalMap.get(id)` を渡して初期値を焼き込むが、
   * これが `init()` より前に走るとホームを含む全ワークツリーが `autoApproval=false`
   * でサブウィンドウ側に入り、以後 `sub-try-auto-approve` を受けても
   * 「自動承認 OFF」として即 `approved=false` を返す（トグルし直すまで戻らない）。
   * 呼び出し側は復元処理より前にこれを呼ぶこと。冪等。
   */
  function restoreFromSettings() {
    for (const wt of deps.settings.value.worktrees) {
      if (wt.autoApproval === true) {
        autoApprovalMap.set(wt.id, true);
      }
    }
  }

  async function init() {
    restoreFromSettings();

    // notify-worktree → 自動承認チェック
    await listen<NotifyWorktreeEvent>("notify-worktree", async (event) => {
      const { worktree_name: worktreeName, kind } = event.payload;

      // このリスナーが扱うのは「承認待ちかもしれない通知」だけ。フィルタをすべての
      // async 処理の前に置く。
      //
      // **許可リスト方式にしている（#140）。** kind と event_kind を統合して種別が
      // 7値へ増えたため、「completed と hook 以外はすべて承認候補」という除外方式だと
      // `worktree.message` などが承認待ち扱いになり、AI 判定ループが走ってしまう。
      if (kind !== "approval" && kind !== "general") return;

      const wt = deps.worktrees.value.find((w) => w.name === worktreeName);
      if (!wt) return;
      if (!autoApprovalMap.get(wt.id)) return;

      // tray は**イベント単位**の属性なので、ここで値を取り出して以降の判定へ持ち回る。
      // ワークツリー単位のラッチに置くと、判定中に届いた `tray: false` のイベントが
      // 直前の明示通知まで抑制してしまう（#168）。
      await handleNotify(wt, trayOf(event.payload));
    });

    // サブウィンドウからの自動承認結果 → 拒否時のみ通知
    await listen<{
      worktreeId: string;
      approved: boolean;
      command?: string;
      tray?: boolean;
      /** サブウィンドウが AI 判定中に預かったイベントの tray（あれば判定結果とは別に 1 件提示する・#168） */
      pendingTray?: boolean;
    }>(
      "sub-auto-approve-result",
      async (event) => {
        const { worktreeId: wid, approved, command, pendingTray } = event.payload;
        // tray は sub-try-auto-approve で渡した値がそのまま返ってくる（イベント単位・#168）
        const tray = trayOf(event.payload);
        logDebug(
          `[AutoApproval] sub-auto-approve-result worktreeId=${wid} approved=${approved} tray=${tray} pendingTray=${pendingTray ?? "none"} command=${command ?? "none"}`
        );
        if (command) {
          deps.lastJudgedCommandMap.set(wid, command);
        }
        const focused = deps.isWorktreeFocused(wid);
        let count = 0;
        if (shouldNotifyAfterJudge({ approved, focused, tray })) count++;
        if (pendingTray !== undefined && shouldNotifyAfterJudge({ approved: false, focused, tray: pendingTray })) {
          count++;
        }
        await notifyApproval(wid, deps.worktrees.value.find((w) => w.id === wid)?.name, count);
      },
    );

    // サブウィンドウからの自動承認バッジクリック → ダイアログ表示
    await listen<{ worktreeId: string }>("sub-click-auto-approval", (event) => {
      deps.onClickAutoApproval(event.payload.worktreeId);
    });

    // トレイポップアップからの自動承認バッジクリック → ダイアログ表示
    await listen<{ worktreeId: string }>("tray-click-auto-approval", (event) => {
      deps.onClickAutoApproval(event.payload.worktreeId);
    });

    // トレイポップアップからのAI判定キャンセル
    await listen<{ worktreeId: string }>("tray-cancel-ai-judging", (event) => {
      onCancelAiJudging(event.payload.worktreeId);
    });
  }

  return { autoApprovalMap, aiJudgingWorktrees, onToggleAutoApproval, onCancelAiJudging, restoreFromSettings, init };
}
