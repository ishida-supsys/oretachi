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

  /** 承認待ちとしてユーザーに提示する（バッジ + 通知音 + OS通知） */
  async function notifyApproval(worktreeId: string, worktreeName: string | undefined) {
    deps.addNotification(worktreeId, "approval");
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
      // ここで捨てると明示 notify_worktree が黙って消える。判定完了後に提示する（#168）
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
    if (
      shouldNotifyAfterJudge({
        approved: loopResult.approved,
        focused: deps.isWorktreeFocused(wt.id),
        tray,
      })
    ) {
      logDebug(`[AutoApproval] local: not approved → addNotification(${wt.id})`);
      await notifyApproval(wt.id, wt.name);
    }

    // 判定中に預かった分。判定を回さなかったイベントなので approved 扱いにはしない。
    const pending = takePendingNotify(pendingNotify, wt.id);
    if (
      pending &&
      shouldNotifyAfterJudge({ approved: false, focused: deps.isWorktreeFocused(wt.id), tray: pending.tray })
    ) {
      logDebug(`[AutoApproval] flush queued notification for ${wt.id}`);
      await notifyApproval(wt.id, wt.name);
    }
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
    await listen<{ worktreeId: string; approved: boolean; command?: string; tray?: boolean }>(
      "sub-auto-approve-result",
      async (event) => {
        const { worktreeId: wid, approved, command } = event.payload;
        // tray は sub-try-auto-approve で渡した値がそのまま返ってくる（イベント単位・#168）
        const tray = trayOf(event.payload);
        logDebug(
          `[AutoApproval] sub-auto-approve-result worktreeId=${wid} approved=${approved} tray=${tray} command=${command ?? "none"}`
        );
        if (command) {
          deps.lastJudgedCommandMap.set(wid, command);
        }
        if (shouldNotifyAfterJudge({ approved, focused: deps.isWorktreeFocused(wid), tray })) {
          await notifyApproval(wid, deps.worktrees.value.find((w) => w.id === wid)?.name);
        }
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
