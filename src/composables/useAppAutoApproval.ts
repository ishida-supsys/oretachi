import { reactive } from "vue";
import { listen, emitTo } from "@tauri-apps/api/event";
import { logDebug } from "../utils/log";
import { runApprovalLoop, cancelApproval } from "../utils/autoApproval";
import type { TerminalForApproval } from "../utils/autoApproval";
import type { Ref } from "vue";
import type { Worktree } from "../types/worktree";
import type { AppSettings } from "../types/settings";
import type { NotificationKind, NotifyWorktreeEvent } from "./useNotifications";
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
  addNotification: (id: string, kind: NotificationKind) => void;
  isWorktreeFocused: (id: string) => boolean;
  onClickAutoApproval: (id: string) => void;
  playSoundForKind: (kind: NotificationKind) => void;
  sendOsNotification: (name: string, title: string) => Promise<void>;
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
   */
  async function notifyApproval(worktreeId: string, worktreeName: string | undefined, count: number) {
    if (count <= 0) return;
    for (let i = 0; i < count; i++) deps.addNotification(worktreeId, "approval");
    deps.playSoundForKind("approval");
    if (worktreeName) await deps.sendOsNotification(worktreeName, deps.t("notification.titleApproval"));
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
        return [{ id: t.id, getTerminal: () => ref.getTerminal(), write: (d: string) => ref.write(d) }];
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

  async function init() {
    // 保存された自動承認状態を復元
    for (const wt of deps.settings.value.worktrees) {
      if (wt.autoApproval === true) {
        autoApprovalMap.set(wt.id, true);
      }
    }

    // notify-worktree → 自動承認チェック
    await listen<NotifyWorktreeEvent>("notify-worktree", async (event) => {
      const { worktree_name: worktreeName, kind } = event.payload;

      // hook/completed はこのリスナーでは不要。フィルタをすべての async 処理の前に置く
      if (kind === "completed" || kind === "hook") return;

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

  return { autoApprovalMap, aiJudgingWorktrees, onToggleAutoApproval, onCancelAiJudging, init };
}
