import { reactive } from "vue";
import { listen, emitTo } from "@tauri-apps/api/event";
import { debug } from "@tauri-apps/plugin-log";
import { runApprovalLoop, cancelApproval } from "../utils/autoApproval";
import type { TerminalForApproval } from "../utils/autoApproval";
import type { Ref } from "vue";
import type { Worktree } from "../types/worktree";
import type { AppSettings } from "../types/settings";
import type { NotificationKind } from "./useNotifications";
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

  async function init() {
    // 保存された自動承認状態を復元
    for (const wt of deps.settings.value.worktrees) {
      if (wt.autoApproval === true) {
        autoApprovalMap.set(wt.id, true);
      }
    }

    // notify-worktree → 自動承認チェック
    await listen<{ worktree_name: string; kind: string }>("notify-worktree", async (event) => {
      const { worktree_name: worktreeName, kind } = event.payload;

      // hook/completed はこのリスナーでは不要。フィルタをすべての async 処理の前に置く
      if (kind === "completed" || kind === "hook") return;

      const wt = deps.worktrees.value.find((w) => w.name === worktreeName);
      if (!wt) return;
      if (!autoApprovalMap.get(wt.id)) return;

      debug(
        `[AutoApproval] notify-worktree received worktreeName=${worktreeName} resolved=${wt.id} autoApproval=true`
      );

      if (aiJudgingWorktrees.has(wt.id)) {
        debug(`[AutoApproval] already in progress for ${wt.id}, skipping`);
        return;
      }

      if (deps.isDetached(wt.id)) {
        debug(`[AutoApproval] delegating to sub-window ${wt.id}`);
        await emitTo(`sub-${wt.id}`, "sub-try-auto-approve", {
          additionalPrompt: deps.autoApprovalPromptMap.get(wt.id) ?? "",
        });
        return;
      }

      debug(`[AutoApproval] local terminals check, count=${wt.terminals.length}`);
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
      if (!loopResult.approved && !deps.isWorktreeFocused(wt.id)) {
        await debug(`[AutoApproval] local: not approved → addNotification(${wt.id})`);
        deps.addNotification(wt.id, "approval");
        deps.playSoundForKind("approval");
        await deps.sendOsNotification(wt.name, deps.t("notification.titleApproval"));
      }
    });

    // サブウィンドウからの自動承認結果 → 拒否時のみ通知
    await listen<{ worktreeId: string; approved: boolean; command?: string }>(
      "sub-auto-approve-result",
      async (event) => {
        const { worktreeId: wid, approved, command } = event.payload;
        await debug(
          `[AutoApproval] sub-auto-approve-result worktreeId=${wid} approved=${approved} command=${command ?? "none"}`
        );
        if (command) {
          deps.lastJudgedCommandMap.set(wid, command);
        }
        if (!approved && !deps.isWorktreeFocused(wid)) {
          deps.addNotification(wid, "approval");
          deps.playSoundForKind("approval");
          const wtName = deps.worktrees.value.find((w) => w.id === wid)?.name;
          if (wtName) await deps.sendOsNotification(wtName, deps.t("notification.titleApproval"));
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
