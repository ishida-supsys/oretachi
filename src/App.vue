<script setup lang="ts">
import { ref, reactive, nextTick, onMounted, computed, watch } from "vue";
import { renderToDataUrl } from "./composables/useTerminalThumbnail";
import { isDirty, clearDirty } from "./composables/usePtyDispatcher";
import { listen, emitTo } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import TerminalView from "./components/TerminalView.vue";
import FrameContainer from "./components/FrameContainer.vue";
import WorktreeHeader from "./components/WorktreeHeader.vue";
import HomeView from "./components/HomeView.vue";
import SettingsView from "./components/SettingsView.vue";
import AddWorktreeDialog from "./components/AddWorktreeDialog.vue";
import AddTaskDialog from "./components/AddTaskDialog.vue";
import RemoveWorktreeDialog from "./components/RemoveWorktreeDialog.vue";
import IdeSelectDialog from "./components/IdeSelectDialog.vue";
import HotkeyCharDialog from "./components/HotkeyCharDialog.vue";
import AutoApprovalPromptDialog from "./components/AutoApprovalPromptDialog.vue";
import TrayButton from "./components/TrayButton.vue";
import { message } from "@tauri-apps/plugin-dialog";
import { invoke } from "@tauri-apps/api/core";
import { useIdeSelect } from "./composables/useIdeSelect";
import { useSettings } from "./composables/useSettings";
import { useWorktrees } from "./composables/useWorktrees";
import { useI18n } from "vue-i18n";
import { useSubWindows, requestSubWindowLayout } from "./composables/useSubWindows";
import { useCodeReviewWindow } from "./composables/useCodeReviewWindow";
import { useArtifactWindow } from "./composables/useArtifactWindow";
import { useNotifications, sendOsNotification, playSoundForKind, type NotificationKind } from "./composables/useNotifications";
import { useTrayPopup } from "./composables/useTrayPopup";
import { useWindowFocus } from "./composables/useWindowFocus";
import { useTasks } from "./composables/useTasks";
import type { TrayWorktreeData, TrayTerminalData } from "./composables/useTrayPopup";
import type { WorktreeEntry } from "./types/settings";
import { useAddTaskDialog } from "./composables/useAddTaskDialog";
import { useTaskExecution } from "./composables/useTaskExecution";
import { useWorktreeTaskMap } from "./composables/useWorktreeTaskMap";
import { useAutoApprovalPrompt } from "./composables/useAutoApprovalPrompt";
import type { FrameNode } from "./types/frame";
import { useWorktreeFrameBundles } from "./composables/useWorktreeFrameBundles";
import { useCodeReviewChatListener } from "./composables/useCodeReviewLineChat";
import { saveWindowState, StateFlags } from "@tauri-apps/plugin-window-state";
import { useRemoveWorktreeDialog } from "./composables/useRemoveWorktreeDialog";
import { useAutoHotkey } from "./composables/useAutoHotkey";
import { useAppAutoApproval } from "./composables/useAppAutoApproval";
import { useAppHotkeys } from "./composables/useAppHotkeys";
import { useSubWindowEvents } from "./composables/useSubWindowEvents";
import { useShutdownGuard } from "./composables/useShutdownGuard";
import { saveArchive, deleteArchive } from "./composables/useArchivePersistence";
import { cancelApproval } from "./utils/autoApproval";
import { debug } from "@tauri-apps/plugin-log";
import { ask } from "@tauri-apps/plugin-dialog";
import { useUpdater } from "./composables/useUpdater";
import Toast from "primevue/toast";

const { t } = useI18n();

// ウィンドウのフォーカス状態
const { isWindowFocused } = useWindowFocus();
const { checkForUpdate, downloadAndInstall } = useUpdater();

// サブウィンドウフォーカス状態: ワークツリー ID → フォーカス中か (useSubWindowEvents で管理)

type ViewMode = "home" | "settings" | "terminal";

const { settings, loadSettings, scheduleSave } = useSettings();
const { worktrees, loadWorktreesFromSettings, addWorktreePlaceholder, invokeWorktreeAdd, commitWorktree, rollbackWorktree, removeWorktree, reorderWorktree, saveWorktreeOrder, restoreWorktreeOrder, addTerminal, removeTerminal, updateTerminalTitle, saveTerminalSession, loadTerminalSession } = useWorktrees();
const { detachedWorktrees, isDetached, moveToSubWindow, moveToMainWindow, focusSubWindow, unregisterSubWindow, getPendingInitData, clearPendingInitData, getDetachedSessionId, registerTerminalSession, closeAllSubWindows } = useSubWindows();
const { autoApprovalPromptMap, lastJudgedCommandMap, showAutoApprovalPromptDialog, autoApprovalPromptTargetId, restoreFromSettings: restoreAutoApprovalPrompts, onClickAutoApproval, onSaveAutoApprovalPrompt } = useAutoApprovalPrompt(settings, scheduleSave, isDetached);
const { notifications, initNotificationListener, addNotification, clearNotification, purgeStaleNotifications, getNotifiedWorktreeIds, getTotalNotificationCount } = useNotifications();
const { openTrayPopup, closeTrayPopup, getPendingWorktrees, clearPendingWorktrees, setCurrentTrayWorktreeId, isTrayShowingWorktree, focusTrayWindow } = useTrayPopup();
const { closeAllCodeReviewWindows } = useCodeReviewWindow();
const { openArtifactViewer, closeArtifactWindow } = useArtifactWindow();
const { tryAutoAssignHotkey } = useAutoHotkey();
const { removeTask } = useTasks();

const homeViewRef = ref<InstanceType<typeof HomeView> | null>(null);

// 自動承認コマンドを猫に送る
watch(
  () => [...lastJudgedCommandMap.entries()],
  (newEntries, oldEntries) => {
    const oldMap = new Map(oldEntries ?? []);
    for (const [worktreeId, command] of newEntries) {
      if (oldMap.get(worktreeId) !== command) {
        homeViewRef.value?.sendCatTopic(command, 1);
      }
    }
  },
  { deep: true },
);

// HomeView / WorktreeCard 向け: Map<string, number> 形式を維持
const notificationCounts = computed(() => {
  const map = new Map<string, number>();
  for (const [id, entry] of notifications) map.set(id, entry.count);
  return map;
});

const viewMode = ref<ViewMode>("home");
const mainContentAreaRef = ref<HTMLDivElement | null>(null);

// terminalId → 直近コマンドの終了コード (null = 未実行)
const terminalExitCodes = reactive(new Map<number, number>());

// terminalId → AIエージェント稼働中フラグ
const terminalAgentStatus = reactive(new Map<number, boolean>());

// terminalId → Webセッション情報（claude --remote で起動したターミナル）
const terminalWebSessions = reactive(new Map<number, import("./types/terminal").WebSessionInfo>());

// terminalId → worktreeId のマッピング（ターミナルがどのワークツリーに属するか）
const terminalWorktreeMap = new Map<number, string>();

// ワークツリーごとのフレームバンドル管理
const {
  bundles: worktreeFrameBundles,
  activeWorktreeId,
  ensureWorktreeFrame,
  getTerminalRef,
  switchToWorktree,
  onFrameSwitch,
  onFrameClose,
  onFrameTabDrop,
  onFrameTabEdgeDrop,
  onFrameTabReorder,
} = useWorktreeFrameBundles({
  worktrees,
  viewMode,
  terminalWorktreeMap,
  terminalExitCodes,
  terminalAgentStatus,
  terminalWebSessions,
  removeTerminal,
  clearNotification,
});

const { setup: setupCodeReviewChatListener } = useCodeReviewChatListener({
  worktrees,
  terminalAgentStatus,
  isDetached,
  getDetachedSessionId,
  // getTerminalRef でバンドルから引く proxy
  terminalRefs: { get: (id: number) => getTerminalRef(id) } as Map<number, { write(data: string): Promise<void> }>,
  worktreeFrameBundles,
  activeWorktreeId,
  switchToWorktree,
  focusSubWindow,
  focusMainWindow: () => getCurrentWindow().setFocus(),
  isTrayShowingWorktree,
  focusTrayPopup: focusTrayWindow,
});

// terminalId → サムネイル data URL
const thumbnailUrls = reactive(new Map<number, string>());

// worktreeId → 実行待ちスクリプトコマンド文字列
const pendingScripts = new Map<string, string>();

// terminalId → 復元スナップショット（起動時セッション復元用）
const pendingSnapshots = new Map<number, string>();
// 新規追加ターミナル: autoStart を抑制して reparenting 後に手動 startPty するための ID セット
const pendingManualStart = new Set<number>();
// terminalId → { sessionId, snapshot } （サブ→メイン移動時のセッション引き継ぎ用）
const pendingSessionAttach = new Map<number, { sessionId: number; snapshot: string }>();

// ワークツリー追加ダイアログ
const showAddDialog = ref(false);


// 削除中のワークツリー ID セット
const loadingWorktrees = reactive(new Map<string, string>());

const { isWaitingForShutdown, isBusyForShutdown, waitForBusyOperations } = useShutdownGuard(loadingWorktrees);

// サブウィンドウイベント管理
const subWindowEvents = useSubWindowEvents({
  worktrees,
  removeTerminal,
  unregisterSubWindow,
  terminalWorktreeMap,
  thumbnailUrls,
  terminalAgentStatus,
  terminalWebSessions,
  updateTerminalTitle,
  clearNotification,
  requestSubWindowLayout,
});

// 自動承認管理
const autoApproval = useAppAutoApproval({
  worktrees,
  settings,
  scheduleSave,
  isDetached,
  getTerminalRef,
  autoApprovalPromptMap,
  lastJudgedCommandMap,
  addNotification,
  isWorktreeFocused,
  onClickAutoApproval,
  playSoundForKind,
  sendOsNotification,
  t,
});

// 短縮参照
const { autoApprovalMap, aiJudgingWorktrees } = autoApproval;
const { onToggleAutoApproval, onCancelAiJudging } = autoApproval;

// タスク実行 (executeAddWorktree / executeAgentWorktree)
const { executeAddWorktree, executeAgentWorktree, resolveShell, buildPendingCommand, waitForScriptCompletion } =
  useTaskExecution({
    t,
    settings,
    worktrees,
    addWorktreePlaceholder,
    invokeWorktreeAdd,
    commitWorktree,
    rollbackWorktree,
    isDetached,
    getDetachedSessionId,
    tryAutoAssignHotkey,
    terminalAgentStatus,
    getTerminalRef,
    onAddTerminal,
    onMoveToSubWindow,
    loadingWorktrees,
    pendingScripts,
    autoApprovalMap,
    onWebSessionDetected: (terminalId, info) => {
      terminalWebSessions.set(terminalId, info);
      // ターミナルがサブウィンドウに移動済みの場合は同期イベントを送る
      const worktreeId = terminalWorktreeMap.get(terminalId);
      if (worktreeId && isDetached(worktreeId)) {
        emitTo(`sub-${worktreeId}`, "sub-web-session", { terminalId, info });
      }
    },
  });
const { getTooltipText: getWorktreeTaskTooltip } = useWorktreeTaskMap();
const { showAddTaskDialog, rerunTaskId, rerunPrompt, onAddTaskConfirm, onAddTaskCancel } =
  useAddTaskDialog(async (code) => {
    if (code.type === "add_worktree") {
      await executeAddWorktree(code);
    } else if (code.type === "agent_worktree") {
      await executeAgentWorktree(code);
    }
  });

// ワークツリー削除/アーカイブダイアログ
const {
  showRemoveDialog,
  removeTargetWorktree,
  removeBranches,
  removeDirtyFiles,
  onRemoveWorktree,
  onRemoveWorktreeConfirm,
  onArchiveWorktreeConfirm,
  dismissDialog: onRemoveWorktreeDismiss,
} = useRemoveWorktreeDialog({
  loadingWorktrees,
  clearNotification,
  isDetached,
  moveToMainWindow,
  subWindowFocusMap: subWindowEvents.subWindowFocusMap,
  closeArtifactWindow,
  worktreeFrameBundles,
  getTerminalRef,
  terminalWorktreeMap,
  activeWorktreeId,
  goHome,
  homeViewRef,
  t,
});

// IDE 選択
const { showIdeDialog, detectedIdes, openInIde, onIdeSelected } = useIdeSelect();

// ホットキー文字ダイアログ
const showHotkeyCharDialog = ref(false);
const hotkeyCharTargetId = ref("");

// hotkeyChars: Map<worktreeId, char>
const hotkeyChars = computed(() => {
  const map = new Map<string, string>();
  for (const wt of settings.value.worktrees) {
    if (wt.hotkeyChar) map.set(wt.id, wt.hotkeyChar);
  }
  return map;
});

// usedHotkeyChars: 現在のダイアログ対象以外が使っている文字
const usedHotkeyChars = computed(() => {
  const used = new Set<string>();
  for (const wt of settings.value.worktrees) {
    if (wt.hotkeyChar && wt.id !== hotkeyCharTargetId.value) {
      used.add(wt.hotkeyChar);
    }
  }
  return used;
});

async function onTerminalReady(worktreeId: string, terminalId: number) {
  // Vue reactivity 強制更新
  const bundle = worktreeFrameBundles.get(worktreeId);
  if (bundle) {
    const ref = bundle.terminalRefs.get(terminalId);
    if (ref) {
      bundle.terminalRefs.delete(terminalId);
      bundle.terminalRefs.set(terminalId, ref);
    }
  }
  pendingSnapshots.delete(terminalId);
  pendingSessionAttach.delete(terminalId);
  const command = pendingScripts.get(worktreeId);
  if (command) {
    pendingScripts.delete(worktreeId);
    const ref = getTerminalRef(terminalId);
    await ref?.write(command);
  }
}

async function switchToTerminal(terminalId: number) {
  // detached ワークツリーのターミナルはサブウィンドウにフォーカス
  const worktreeId = terminalWorktreeMap.get(terminalId);
  if (worktreeId) clearNotification(worktreeId);
  if (worktreeId && isDetached(worktreeId)) {
    await focusSubWindow(worktreeId);
    await emitTo(`sub-${worktreeId}`, "sub-focus-terminal", { terminalId });
    return;
  }
  if (!worktreeId) return;

  debug(`[Terminal] switchToTerminal terminalId=${terminalId} worktreeId=${worktreeId}`);

  const bundle = worktreeFrameBundles.get(worktreeId);
  if (bundle) {
    const leaf = bundle.frame.getAllLeafs().find((l) => l.terminalIds.includes(terminalId));
    if (leaf) {
      bundle.frame.setActiveTerminal(leaf.id, terminalId);
      bundle.frame.lastFocusedLeafId.value = leaf.id;
    }
  }

  viewMode.value = "terminal";
  activeWorktreeId.value = worktreeId;
  await nextTick();
  debug(`[Terminal] switchToTerminal after nextTick terminalId=${terminalId}`);

  if (bundle) {
    bundle.frame.mountTerminalsToHosts();
    debug(`[Terminal] switchToTerminal after mountTerminalsToHosts terminalId=${terminalId}`);
    const term = bundle.terminalRefs.get(terminalId);
    if (term) {
      await term.handleTabActivated();
      debug(`[Terminal] switchToTerminal after handleTabActivated terminalId=${terminalId}`);
      term.focus();
      // 安全策: flexレイアウト確定が遅延する場合に備えた再fit
      setTimeout(() => {
        debug(`[Terminal] switchToTerminal setTimeout re-fit terminalId=${terminalId}`);
        term.handleTabActivated();
      }, 150);
    } else {
      debug(`[Terminal] switchToTerminal term ref not found terminalId=${terminalId}`);
    }
  }
}

function goHome() {
  viewMode.value = "home";
  activeWorktreeId.value = null;
}

function goSettings() {
  viewMode.value = "settings";
  activeWorktreeId.value = null;
}

function onTabBarDrag(e: MouseEvent) {
  if ((e.target as HTMLElement).closest("button")) return;
  getCurrentWindow().startDragging();
}

async function minimizeWindow() {
  await getCurrentWindow().minimize();
}

async function toggleMaximizeWindow() {
  await getCurrentWindow().toggleMaximize();
}

async function closeWindow() {
  await getCurrentWindow().close();
}

async function onAddTerminal(worktreeId: string) {
  clearNotification(worktreeId);
  if (isDetached(worktreeId)) {
    await handleSubAddTerminalRequest(worktreeId);
    return;
  }

  const terminal = addTerminal(worktreeId);
  pendingManualStart.add(terminal.id);
  terminalWorktreeMap.set(terminal.id, worktreeId);
  debug(`[Terminal] onAddTerminal worktreeId=${worktreeId} terminalId=${terminal.id}`);

  // バンドルがなければ作成（ワークツリー追加直後など）
  if (!worktreeFrameBundles.has(worktreeId)) {
    ensureWorktreeFrame(worktreeId);
  }

  const bundle = worktreeFrameBundles.get(worktreeId)!;
  bundle.terminalEntries.set(terminal.id, { id: terminal.id, title: terminal.title, sessionId: 0, snapshot: "" });

  const leafId = bundle.frame.lastFocusedLeafId.value || bundle.frame.getAllLeafs()[0]?.id;
  debug(`[Terminal] onAddTerminal leafId=${leafId ?? "null"} terminalId=${terminal.id}`);
  if (leafId) {
    bundle.frame.returnAllToOffscreen();
    bundle.frame.addTerminalToLeaf(leafId, terminal.id);
    bundle.frame.lastFocusedLeafId.value = leafId;
  } else {
    bundle.frame.initLayout([terminal.id]);
    bundle.frame.lastFocusedLeafId.value = bundle.frame.getAllLeafs()[0]?.id ?? "";
  }

  viewMode.value = "terminal";
  activeWorktreeId.value = worktreeId;

  await nextTick();
  debug(`[Terminal] onAddTerminal after nextTick terminalId=${terminal.id}`);

  bundle.frame.mountTerminalsToHosts();
  debug(`[Terminal] onAddTerminal after mountTerminalsToHosts terminalId=${terminal.id}`);
  const term = bundle.terminalRefs.get(terminal.id);
  if (term) {
    await term.handleTabActivated();
    debug(`[Terminal] onAddTerminal after handleTabActivated terminalId=${terminal.id}`);
    pendingManualStart.delete(terminal.id);
    await term.startPty();
    debug(`[Terminal] onAddTerminal after startPty terminalId=${terminal.id}`);
    term.focus();
    // 安全策: flexレイアウト確定が遅延する場合に備えた再fit
    setTimeout(() => {
      debug(`[Terminal] onAddTerminal setTimeout re-fit terminalId=${terminal.id}`);
      term.handleTabActivated();
    }, 150);
  } else {
    pendingManualStart.delete(terminal.id);
    debug(`[Terminal] onAddTerminal term ref not found terminalId=${terminal.id}`);
  }
}


function onTerminalTitleChange(worktreeId: string, terminalId: number, title: string) {
  updateTerminalTitle(worktreeId, terminalId, title);
  const bundle = worktreeFrameBundles.get(worktreeId);
  const entry = bundle?.terminalEntries.get(terminalId);
  if (entry) entry.title = title;
}

async function onSessionExit(worktreeId: string, terminalId: number) {
  const bundle = worktreeFrameBundles.get(worktreeId);
  if (bundle) {
    bundle.frame.handleTerminalExit(terminalId);
  }
}


async function onOpenInIde(worktreeId: string) {
  const worktree = worktrees.value.find((w) => w.id === worktreeId);
  if (!worktree) return;
  await openInIde(worktree.path, { worktreeId: worktree.id, worktreeName: worktree.name });
}

async function onOpenArtifacts(worktreeId: string) {
  const worktree = worktrees.value.find((w) => w.id === worktreeId);
  if (!worktree) return;
  await openArtifactViewer(worktree.id, worktree.name);
}

async function onAddWorktreeConfirm(entry: WorktreeEntry, sourceBranch?: string) {
  // ダイアログを即閉じ、一覧に仮エントリを表示
  showAddDialog.value = false;
  addWorktreePlaceholder(entry);
  loadingWorktrees.set(entry.id, t("creatingText"));

  try {
    const lfsSkipped = await invokeWorktreeAdd(entry, sourceBranch);

    // 成功時: 設定に永続化
    commitWorktree(entry);
    tryAutoAssignHotkey(entry.id);

    // デフォルト: 自動承認
    if (settings.value.worktreeDefaults?.autoApproval) {
      autoApprovalMap.set(entry.id, true);
      const wtEntry = settings.value.worktrees.find((w) => w.id === entry.id);
      if (wtEntry) wtEntry.autoApproval = true;
    }

    // gitignoreコピー対象があればコピー実行（スクリプト実行前）
    const repo = settings.value.repositories.find((r) => r.id === entry.repositoryId);
    if (repo?.copyTargets?.length) {
      try {
        await invoke("copy_gitignore_targets", {
          repoPath: repo.path,
          worktreePath: entry.path,
          targets: repo.copyTargets,
        });
      } catch (e) {
        await message(t("copyTargetsFailed", { error: e }), { kind: "warning" });
      }
    }

    // Claude Code通知フックが設定されていれば settings.local.json を生成
    if (repo?.notificationHooks?.length) {
      try {
        await invoke("write_claude_hooks", {
          worktreePath: entry.path,
          worktreeName: entry.name,
          hooks: repo.notificationHooks,
        });
      } catch (e) {
        await message(t("claudeHooksFailed", { error: e }), { kind: "warning" });
      }
    }

    // パッケージマネージャーinstall・スクリプトをペンディング登録
    if (repo) {
      const pending = buildPendingCommand(repo, entry);
      if (pending) {
        pendingScripts.set(entry.id, pending);
      }
    }

    // ワークツリー作成後、自動でターミナルを1つ追加
    await onAddTerminal(entry.id);

    // パッケージマネージャーinstall・スクリプトの完了を待つ
    if (repo?.packageManager || repo?.execScript) {
      await waitForScriptCompletion(entry.id);
    }

    // デフォルト: サブウィンドウで開く
    if (settings.value.worktreeDefaults?.openInSubWindow) {
      await onMoveToSubWindow(entry.id);
    }

    if (lfsSkipped) {
      await message(t("lfsWarning"), { kind: "warning" });
    }
  } catch (e) {
    rollbackWorktree(entry.id);
    await message(t("worktreeCreateFailed", { error: e }), { kind: "error" });
  } finally {
    loadingWorktrees.delete(entry.id);
  }
}

// ─── ────────────────────────────────────────────────────────────────────────

async function handleSubAddTerminalRequest(worktreeId: string) {
  const worktree = worktrees.value.find((w) => w.id === worktreeId);
  if (!worktree) return;

  const terminal = addTerminal(worktreeId);
  terminalWorktreeMap.set(terminal.id, worktreeId);

  const sid = await invoke<number>("pty_spawn", {
    rows: 24, cols: 80, shell: resolveShell(worktreeId) ?? null, cwd: worktree.path,
  });

  registerTerminalSession(worktreeId, terminal.id, sid);

  await emitTo(`sub-${worktreeId}`, "sub-add-terminal-response", {
    terminalId: terminal.id,
    sessionId: sid,
    title: terminal.title,
  });
}

async function onMoveToSubWindow(worktreeId: string) {
  const worktree = worktrees.value.find((w) => w.id === worktreeId);
  if (!worktree) return;

  // フレームレイアウトをシリアライズ
  const bundle = worktreeFrameBundles.get(worktreeId);
  const layout = bundle ? (JSON.parse(JSON.stringify(bundle.frame.root.value)) as FrameNode) : undefined;

  // 各ターミナルの sessionId とスナップショットを収集
  const terminals = worktree.terminals.map((t) => {
    const termRef = getTerminalRef(t.id);
    const sessionId = termRef?.sessionId ?? null;
    const snapshot = sessionId !== null ? (termRef?.serializeBuffer() ?? "") : "";
    return {
      id: t.id,
      title: t.title,
      sessionId: sessionId ?? 0,
      snapshot,
      isAiAgent: terminalAgentStatus.get(t.id) ?? false,
    };
  }).filter((t) => t.sessionId !== 0);

  // 各ターミナルの PTY を detach (アンマウント時に kill させない)
  for (const t of worktree.terminals) {
    const termRef = getTerminalRef(t.id);
    termRef?.detach();
  }

  // アクティブワークツリーがこのワークツリーの場合はホームへ
  if (activeWorktreeId.value === worktreeId) {
    goHome();
  }

  await moveToSubWindow(worktreeId, worktree.name, terminals, autoApprovalMap.get(worktreeId) ?? false, false, worktree.path, layout, worktree.branchName, autoApprovalPromptMap.get(worktreeId), worktree.repositoryName);

  // バンドルをクリーンアップ
  worktreeFrameBundles.delete(worktreeId);
}

function isWorktreeFocused(worktreeId: string): boolean {
  if (isDetached(worktreeId)) {
    return subWindowEvents.subWindowFocusMap.get(worktreeId) === true;
  }
  return isWindowFocused.value && viewMode.value === "terminal" && activeWorktreeId.value === worktreeId;
}

// メインウィンドウでワークツリーがフォーカス状態になったら通知をクリア
watch(
  () => isWindowFocused.value && viewMode.value === "terminal" && activeWorktreeId.value,
  (worktreeId) => {
    if (worktreeId) {
      clearNotification(worktreeId);
    }
  },
);


async function onMoveToMainWindow(worktreeId: string) {
  // サブウィンドウからレイアウトとターミナル情報を取得（destroy前に）
  const { layout: savedLayout, terminals: savedTerminals } = isDetached(worktreeId)
    ? await subWindowEvents.getSubWindowLayout(worktreeId)
    : { layout: null, terminals: [] };

  debug(`[MoveToMain] start worktreeId=${worktreeId} savedTerminals=${JSON.stringify(savedTerminals.map(t => ({ id: t.id, sessionId: t.sessionId })))}`);

  // ★ moveToMainWindow の前にデータを準備（Vue再描画より先にセット）
  // pendingSessionAttach はプレーンMap → Vue再描画をトリガーしない
  for (const t of savedTerminals) {
    if (t.sessionId) {
      pendingSessionAttach.set(t.id, { sessionId: t.sessionId, snapshot: t.snapshot });
    }
    if (t.isAiAgent) {
      terminalAgentStatus.set(t.id, true);
    }
  }
  debug(`[MoveToMain] pendingSessionAttach set for terminalIds=[${[...pendingSessionAttach.keys()]}]`);

  // ensureWorktreeFrame は bundles.set() で再描画をトリガーするが、
  // isDetached がまだ true なので TerminalView は作成されない
  ensureWorktreeFrame(worktreeId, savedLayout ?? undefined);
  debug(`[MoveToMain] ensureWorktreeFrame done, calling moveToMainWindow`);

  await moveToMainWindow(worktreeId);
  debug(`[MoveToMain] moveToMainWindow done, isDetached=${isDetached(worktreeId)}`);

  subWindowEvents.subWindowFocusMap.delete(worktreeId);
}

async function onTrayButtonClick() {
  purgeStaleNotifications(new Set(worktrees.value.map((w) => w.id)));
  const worktreeIds = getNotifiedWorktreeIds();
  if (worktreeIds.length === 0) {
    if (settings.value.focusMainOnEmptyTray) {
      const win = getCurrentWindow();
      await win.show();
      await win.unminimize();
      await win.setFocus();
    }
    return;
  }

  const worktreeDataList: TrayWorktreeData[] = [];

  for (const worktreeId of worktreeIds) {
    const worktree = worktrees.value.find((w) => w.id === worktreeId);
    if (!worktree) continue;

    if (isDetached(worktreeId)) {
      // サブウィンドウのレイアウト情報を要求
      const layoutData = await requestSubWindowLayout(worktreeId) as { layout: FrameNode | null; terminals: TrayTerminalData[]; windowSize?: { width: number; height: number } } | null;

      worktreeDataList.push({
        worktreeId,
        worktreeName: worktree.name,
        worktreePath: worktree.path,
        isDetached: true,
        layout: (layoutData?.layout ?? null) as import("./types/frame").FrameNode | null,
        terminals: layoutData?.terminals ?? [],
        windowSize: layoutData?.windowSize,
        branchName: worktree.branchName,
        hotkeyChar: hotkeyChars.value.get(worktreeId),
        autoApproval: autoApprovalMap.get(worktreeId) ?? false,
        aiJudging: aiJudgingWorktrees.has(worktreeId),
      });
    } else {
      // メインウィンドウのターミナル情報を収集
      const mainBundle = worktreeFrameBundles.get(worktreeId);
      const mainLayout = mainBundle
        ? (JSON.parse(JSON.stringify(mainBundle.frame.root.value)) as FrameNode)
        : null;

      const terminals: TrayTerminalData[] = worktree.terminals.map((t) => {
        const termRef = getTerminalRef(t.id);
        const sessionId = termRef?.sessionId ?? null;
        const snapshot = sessionId !== null ? (termRef?.serializeBuffer(300) ?? "") : "";
        const termObj = termRef?.getTerminal();
        return {
          id: t.id,
          title: t.title,
          sessionId: sessionId ?? 0,
          snapshot,
          rows: termObj?.rows ?? 24,
          cols: termObj?.cols ?? 80,
        };
      }).filter((t) => t.sessionId !== 0);

      // フレーム領域のサイズを取得（ヘッダー除く）
      let frameAreaEl: Element | null = document.querySelector(`[data-frame-area="${worktreeId}"]`);
      let rect = frameAreaEl?.getBoundingClientRect();
      // 非アクティブワークツリーは v-show で非表示のため rect が 0x0 になる
      // 全フレームは absolute inset-0 で同サイズなので、アクティブワークツリーの領域で代替
      if (!rect || rect.width === 0 || rect.height === 0) {
        frameAreaEl = document.querySelector(`[data-frame-area="${activeWorktreeId.value}"]`);
        rect = frameAreaEl?.getBoundingClientRect();
      }
      let mainWindowSize: { width: number; height: number } | undefined;
      // sizeFromContainer: 親コンテナからのフォールバックサイズを使ったかどうか
      let sizeFromContainer = false;
      if (rect && rect.width > 0 && rect.height > 0) {
        mainWindowSize = { width: Math.round(rect.width), height: Math.round(rect.height) };
      } else {
        // activeWorktreeId が null（ホーム/設定画面）の場合:
        // メインコンテンツ領域を計測（= WorktreeHeader + フレーム領域と同サイズ）
        const containerRect = mainContentAreaRef.value?.getBoundingClientRect();
        if (containerRect && containerRect.width > 0 && containerRect.height > 0) {
          mainWindowSize = { width: Math.round(containerRect.width), height: Math.round(containerRect.height) };
          sizeFromContainer = true;
        }
      }

      worktreeDataList.push({
        worktreeId,
        worktreeName: worktree.name,
        worktreePath: worktree.path,
        isDetached: sizeFromContainer,  // コンテナサイズの場合はヘッダー加算をスキップ
        layout: mainLayout,
        terminals,
        windowSize: mainWindowSize,
        branchName: worktree.branchName,
        hotkeyChar: hotkeyChars.value.get(worktreeId),
        autoApproval: autoApprovalMap.get(worktreeId) ?? false,
        aiJudging: aiJudgingWorktrees.has(worktreeId),
        autoApprovalPrompt: autoApprovalPromptMap.get(worktreeId) ?? '',
        lastJudgedCommand: lastJudgedCommandMap.get(worktreeId) ?? '',
      });
    }
  }

  if (worktreeDataList.length === 0) return;

  await openTrayPopup(worktreeDataList);
}

function onFrameAddTerminal(wid: string, leafId: string) {
  const bundle = worktreeFrameBundles.get(wid);
  if (bundle) bundle.frame.lastFocusedLeafId.value = leafId;
  onAddTerminal(wid);
}


async function onFocusSubWindow(worktreeId: string) {
  clearNotification(worktreeId);
  await focusSubWindow(worktreeId);
}

async function onFocusAllSubWindows() {
  for (const worktreeId of detachedWorktrees) {
    await focusSubWindow(worktreeId);
  }
}

function onSetHotkeyChar(worktreeId: string) {
  hotkeyCharTargetId.value = worktreeId;
  showHotkeyCharDialog.value = true;
}

function onHotkeyCharConfirm(worktreeId: string, char: string) {
  const wt = settings.value.worktrees.find((w) => w.id === worktreeId);
  if (wt) {
    wt.hotkeyChar = char;
    scheduleSave();
  }
  showHotkeyCharDialog.value = false;
}

function onHotkeyCharClear(worktreeId: string) {
  const wt = settings.value.worktrees.find((w) => w.id === worktreeId);
  if (wt) {
    wt.hotkeyChar = undefined;
    scheduleSave();
  }
  showHotkeyCharDialog.value = false;
}

// ─── composable instantiation ────────────────────────────────────────────────

const hotkeys = useAppHotkeys({
  settings,
  loadSettings,
  activeWorktreeId,
  worktreeFrameBundles,
  viewMode,
  worktrees,
  isDetached,
  switchToWorktree,
  focusSubWindow,
  onAddTerminal,
  showAddTaskDialog,
  goHome,
  onTrayButtonClick,
});


onMounted(async () => {
  await loadSettings();
  await getCurrentWindow().setAlwaysOnTop(settings.value.alwaysOnTop);
  loadWorktreesFromSettings();
  restoreAutoApprovalPrompts();

  // 保存されたサブウィンドウを復元
  const savedDetachedIds = settings.value.detachedWorktreeIds ?? [];

  // メインウィンドウのターミナルセッションを復元（detachedでないワークツリー）
  for (const wt of worktrees.value) {
    if (savedDetachedIds.includes(wt.id)) continue;
    try {
      const session = await loadTerminalSession(wt.id);
      if (session && session.terminals.length > 0) {
        for (const saved of session.terminals) {
          const terminal = addTerminal(wt.id);
          updateTerminalTitle(wt.id, terminal.id, saved.title);
          terminalWorktreeMap.set(terminal.id, wt.id);
          if (saved.buffer) {
            pendingSnapshots.set(terminal.id, saved.buffer);
          }
        }
      }
    } catch {
      // セッション読み込み失敗は無視
    }
    // ターミナル追加後にバンドルを作成
    ensureWorktreeFrame(wt.id);
  }

  // サブウィンドウからのホームタブ要求
  await listen("go-home", () => {
    goHome();
  });

  // MCPからのタスク追加
  await listen<{ prompt: string; remote_exec: boolean }>("mcp-add-task", (event) => {
    onAddTaskConfirm(event.payload.prompt, event.payload.remote_exec);
  });

  // MCPからのワークツリークローズ（アーカイブ）
  await listen<{ worktree_id: string; worktree_name: string; merge_to: string; delete_branch: boolean; force_branch: boolean }>("mcp-close-worktree", async (event) => {
    const { worktree_id, merge_to, delete_branch, force_branch } = event.payload;
    const worktree = worktrees.value.find((w) => w.id === worktree_id);
    if (!worktree) return;

    clearNotification(worktree_id);
    loadingWorktrees.set(worktree_id, t("archivingText"));
    try {
      // アーカイブをDBに保存（git操作前に実行）
      await saveArchive({
        id: worktree.id,
        name: worktree.name,
        repository_id: worktree.repositoryId,
        repository_name: worktree.repositoryName,
        path: worktree.path,
        branch_name: worktree.branchName,
        archived_at: Date.now(),
      });

      // detachedの場合はメインウィンドウに戻してからターミナルを停止する
      // （サブウィンドウのターミナルrefはメインウィンドウから参照できないため）
      if (isDetached(worktree_id)) {
        await moveToMainWindow(worktree_id);
        subWindowEvents.subWindowFocusMap.delete(worktree_id);
      }

      // ターミナルを停止（git worktree remove前にファイルハンドルを解放するため）
      const bundle = worktreeFrameBundles.get(worktree_id);
      for (const terminal of [...worktree.terminals]) {
        const term = bundle?.terminalRefs.get(terminal.id) ?? getTerminalRef(terminal.id);
        if (term?.isRunning) await term.kill();
        terminalWorktreeMap.delete(terminal.id);
      }
      worktreeFrameBundles.delete(worktree_id);

      await removeWorktree(worktree_id, {
        mergeTo: merge_to || undefined,
        deleteBranch: delete_branch,
        forceBranch: force_branch,
      });

      // git操作成功後にUIをクリーンアップ
      await closeArtifactWindow(worktree_id);
      await cancelApproval(worktree_id);
      if (activeWorktreeId.value === worktree_id) goHome();
    } catch {
      // 失敗時: ワークツリーパスの有無でケースを判定する
      const pathStillExists = await invoke<boolean>("path_exists", { path: worktree.path }).catch(() => false);
      if (pathStillExists) {
        // git_worktree_remove が失敗（ワークツリーが残っている）→ アーカイブをロールバック
        await deleteArchive(worktree_id);
      } else {
        // ワークツリーは削除済み（ブランチ削除のみ失敗）→ UIクリーンアップは実行する
        await closeArtifactWindow(worktree_id);
        await cancelApproval(worktree_id);
        if (activeWorktreeId.value === worktree_id) goHome();
      }
    } finally {
      loadingWorktrees.delete(worktree_id);
    }
  });

  // サブウィンドウ準備完了 → init データをイベントで送信（サブウィンドウ復元より前に登録必須）
  await listen<{ worktreeId: string }>("sub-ready", async (event) => {
    const { worktreeId } = event.payload;
    // 新しいサブウィンドウは作成時にフォーカスされている
    subWindowEvents.subWindowFocusMap.set(worktreeId, true);
    const initData = getPendingInitData(worktreeId);
    if (initData) {
      // このワークツリーに属するターミナルのWebセッション情報を収集
      const webSessions: Record<number, import("./types/terminal").WebSessionInfo> = {};
      for (const t of initData.terminals) {
        const info = terminalWebSessions.get(t.id);
        if (info) webSessions[t.id] = info;
      }
      await emitTo(`sub-${worktreeId}`, "sub-init", {
        worktreeId,
        terminals: initData.terminals,
        autoApproval: initData.autoApproval,
        autoApprovalPrompt: initData.autoApprovalPrompt,
        layout: initData.layout,
        webSessions,
      });
      clearPendingInitData(worktreeId);
    }
  });

  // サブウィンドウとして保存されていたワークツリーを復元
  for (const worktreeId of savedDetachedIds) {
    const wt = worktrees.value.find((w) => w.id === worktreeId);
    if (!wt) continue;

    // セッションファイルからターミナルデータを読み込み
    let subTerminals: { id: number; title: string; sessionId: number; snapshot: string }[] = [];
    try {
      const session = await loadTerminalSession(worktreeId);
      if (session && session.terminals.length > 0) {
        for (const saved of session.terminals) {
          const terminal = addTerminal(worktreeId);
          updateTerminalTitle(worktreeId, terminal.id, saved.title);
          terminalWorktreeMap.set(terminal.id, worktreeId);
          const sid = await invoke<number>("pty_spawn", {
            rows: 24, cols: 80, shell: resolveShell(worktreeId) ?? null, cwd: wt.path,
          });
          subTerminals.push({ id: terminal.id, title: saved.title, sessionId: sid, snapshot: saved.buffer });
        }
      }
    } catch {
      // セッション読み込み失敗は無視
    }

    await moveToSubWindow(wt.id, wt.name, subTerminals, autoApprovalMap.get(wt.id) ?? false, true, wt.path, undefined, wt.branchName, autoApprovalPromptMap.get(wt.id), wt.repositoryName);
  }

  // 通知リスナー初期化 (ワークツリー名 → ID 解決関数と自動承認中は保留するコールバックを渡す)
  await initNotificationListener(
    (name: string) => worktrees.value.find((w) => w.name === name)?.id,
    (id: string, kind: NotificationKind) => {
      if (kind === "completed") return isWorktreeFocused(id);
      return autoApprovalMap.get(id) === true || isWorktreeFocused(id);
    },
    () => settings.value.enableOsNotification === true,
    (worktreeId: string) => {
      if (isDetached(worktreeId)) {
        focusSubWindow(worktreeId);
      } else {
        const wt = worktrees.value.find((w) => w.id === worktreeId);
        if (wt && wt.terminals.length > 0) {
          switchToWorktree(worktreeId);
        }
        getCurrentWindow().setFocus();
      }
    },
    {
      general: t("notification.title"),
      approval: t("notification.titleApproval"),
      completed: t("notification.titleCompleted"),
    },
    () => settings.value.notificationSound,
  );

  // 自動承認リスナーを初期化（notify-worktree, sub-auto-approve-result 等）
  await autoApproval.init();

  // サブウィンドウからの自動承認プロンプト保存
  await listen<{ worktreeId: string; prompt: string }>("sub-save-auto-approval-prompt", (event) => {
    onSaveAutoApprovalPrompt(event.payload.worktreeId, event.payload.prompt);
  });

  // トレイポップアップからの自動承認プロンプト保存
  await listen<{ worktreeId: string; prompt: string }>("tray-save-auto-approval-prompt", (event) => {
    onSaveAutoApprovalPrompt(event.payload.worktreeId, event.payload.prompt);
  });

  // ターミナル追加リクエスト (サブウィンドウから)
  await listen<{ worktreeId: string }>("sub-add-terminal-request", async (event) => {
    await handleSubAddTerminalRequest(event.payload.worktreeId);
  });

  // アーティファクト追加時に自動でビューアを開く
  await listen<{ worktreeId: string; artifactId: string; command: string }>("artifact-changed", async (event) => {
    const { worktreeId: wid, command } = event.payload;
    if (command !== "create") return;
    if (settings.value.worktreeDefaults?.autoOpenArtifact === false) return;
    const wt = worktrees.value.find((w) => w.id === wid);
    if (!wt) return;
    await openArtifactViewer(wt.id, wt.name);
  });

  // サブウィンドウイベントリスナーを初期化
  await subWindowEvents.init();

  // AIエージェントインジケーター: pty-ai-agent-changed を受信して terminalAgentStatus を更新
  await listen<{ sessions: Record<number, boolean> }>("pty-ai-agent-changed", (event) => {
    // sessionId → terminalId の逆引きマップを構築
    const sessionToTerminal = new Map<number, number>();
    for (const [, bundle] of worktreeFrameBundles) {
      for (const [tid, termRef] of bundle.terminalRefs) {
        const sid = termRef?.sessionId;
        if (sid != null) sessionToTerminal.set(sid, tid);
      }
    }
    for (const [sessionIdStr, isAgent] of Object.entries(event.payload.sessions)) {
      const sid = Number(sessionIdStr);
      const tid = sessionToTerminal.get(sid);
      if (tid != null) {
        if (isAgent) {
          terminalAgentStatus.set(tid, true);
        } else {
          terminalAgentStatus.delete(tid);
        }
      }
    }
  });

  await hotkeys.init();

  // トレイポップアップ準備完了 → init データ送信
  await listen("tray-ready", async () => {
    const worktrees = getPendingWorktrees();
    if (worktrees) {
      try {
        await emitTo("tray-popup", "tray-init", { worktrees });
        if (worktrees.length > 0) {
          setCurrentTrayWorktreeId(worktrees[0].worktreeId);
        }
      } catch (e) {
        console.error("tray-init 送信失敗:", e);
      }
      clearPendingWorktrees();
    }
  });

  // トレイポップアップが別ワークツリーに切り替えた通知
  await listen<{ worktreeId: string }>("tray-current-worktree-changed", (event) => {
    setCurrentTrayWorktreeId(event.payload.worktreeId);
  });

  // トレイポップアップからの通知クリア
  await listen<{ worktreeId: string }>("tray-clear-notification", (event) => {
    clearNotification(event.payload.worktreeId);
  });

  // トレイポップアップ閉鎖通知（tray popup自身がdestroyするのでskip）
  await listen("tray-closing", () => {
    closeTrayPopup(true);
  });

  // Code Review チャットボタン → AIエージェントターミナルへ書き込み
  await setupCodeReviewChatListener();

  // 初期ターミナルの worktreeId マッピングを構築
  for (const wt of worktrees.value) {
    for (const t of wt.terminals) {
      terminalWorktreeMap.set(t.id, wt.id);
    }
  }

  // ローカルターミナル用サムネイル生成ループ（pty出力があった場合のみ更新）
  setInterval(() => {
    for (const [wid, bundle] of worktreeFrameBundles) {
      if (isDetached(wid)) continue;
      for (const [id, ref] of bundle.terminalRefs) {
        const sid = ref.sessionId;
        if (sid == null || !isDirty(sid)) continue;
        const terminal = ref.getTerminal();
        if (terminal) {
          const url = renderToDataUrl(terminal);
          if (url && url !== thumbnailUrls.get(id)) thumbnailUrls.set(id, url);
          clearDirty(sid);
        }
      }
    }
  }, 1000);

  // メインウィンドウ閉じ時: 全サブウィンドウを閉じてからアプリ終了
  await getCurrentWindow().onCloseRequested(async (event) => {
    event.preventDefault();
    if (isWaitingForShutdown.value) return; // 二重クリック防止
    isWaitingForShutdown.value = true;

    // ワークツリー操作/タスク実行中の場合は完了まで待機
    if (isBusyForShutdown.value) {
      await waitForBusyOperations();
    }

    // 1. 全ウィンドウの位置・サイズをプラグインで保存
    await saveWindowState(StateFlags.ALL);

    // 2. サブウィンドウ化しているワークツリーIDを設定に保存
    settings.value.detachedWorktreeIds = Array.from(detachedWorktrees);
    await invoke("save_settings", { settings: settings.value });

    // 3. サブウィンドウのターミナルセッションを保存（closeAllSubWindows より前）
    for (const wt of worktrees.value) {
      if (!isDetached(wt.id)) continue;
      try {
        const response = await new Promise<{ terminals: { title: string; buffer: string }[] } | null>((resolve) => {
          const timeout = setTimeout(() => { unlisten(); resolve(null); }, 3000);
          let unlisten = () => {};
          listen<{ worktreeId: string; terminals: { title: string; buffer: string }[] }>(
            "sub-session-save-response",
            (event) => {
              if (event.payload.worktreeId === wt.id) {
                clearTimeout(timeout);
                unlisten();
                resolve({ terminals: event.payload.terminals });
              }
            }
          ).then((fn) => { unlisten = fn; });
          emitTo(`sub-${wt.id}`, "sub-session-save-request", {}).catch(() => {
            clearTimeout(timeout);
            unlisten();
            resolve(null);
          });
        });
        if (response && response.terminals.length > 0) {
          await saveTerminalSession(wt.id, response.terminals);
        }
      } catch {
        // セッション保存失敗は無視
      }
    }

    // 4. メインウィンドウのターミナルセッションを保存
    for (const wt of worktrees.value) {
      if (isDetached(wt.id)) continue;
      try {
        const bundle = worktreeFrameBundles.get(wt.id);
        const terminals = wt.terminals.map((t) => {
          const termRef = bundle?.terminalRefs.get(t.id) ?? getTerminalRef(t.id);
          return { title: t.title, buffer: termRef?.serializeBuffer() ?? "" };
        }).filter((t) => t.buffer !== "");
        if (terminals.length > 0) {
          await saveTerminalSession(wt.id, terminals);
        }
      } catch {
        // セッション保存失敗は無視
      }
    }

    // 5. 既存のシャットダウン処理
    await Promise.all([
      closeTrayPopup(),
      closeAllCodeReviewWindows(),
      closeAllSubWindows(),
    ]);
    await Promise.all(
      [...worktreeFrameBundles.values()].flatMap((bundle) =>
        [...bundle.terminalRefs.values()].filter((term) => term?.isRunning).map((term) => term!.kill()),
      ),
    );
    await getCurrentWindow().destroy();
  });

  // 起動後にアップデートを確認
  setTimeout(async () => {
    const update = await checkForUpdate();
    if (update) {
      const yes = await ask(
        t("update.available", { version: update.version }),
        { title: t("update.title"), kind: "info" }
      );
      if (yes) {
        await downloadAndInstall(update);
      }
    }
  }, 3000);
});

</script>

<template>
  <div
    class="h-screen flex flex-col text-[#cdd6f4] select-none"
    style="background-color: var(--bg-base)"
    :class="[{ 'gaming-border': settings.appearance?.enableGamingBorder }, settings.appearance?.enableGamingBorder ? `gaming-theme-${settings.appearance?.gamingBorderTheme ?? 'gaming'}` : '']"
  >
    <!-- タブバー -->
    <div
      class="flex items-center border-b shrink-0 min-h-0 transition-colors duration-200"
      style="background-color: var(--bg-mantle)"
      :class="isWindowFocused ? 'border-[#cba6f7]/50' : 'opacity-90 border-[#313244]'"
      @mousedown.left="onTabBarDrag"
    >
      <!-- ホームボタン -->
      <button
        class="px-4 text-sm font-semibold tracking-wide shrink-0 py-2 transition-colors"
        :class="viewMode === 'home' ? 'text-[#cba6f7]' : 'text-[#6c7086] hover:text-[#cba6f7]'"
        @click="goHome"
      >
        oretachi
      </button>

      <!-- 設定ボタン -->
      <button
        class="px-3 py-2 text-sm shrink-0 transition-colors"
        :class="viewMode === 'settings' ? 'text-[#cba6f7]' : 'text-[#6c7086] hover:text-[#cba6f7]'"
        title="設定"
        @click="goSettings"
      >
        <span class="pi pi-cog" />
      </button>

      <!-- 区切り線 -->
      <div class="w-px h-5 bg-[#313244] shrink-0 mx-1" />

      <!-- ワークツリー単位のタブ -->
      <div class="flex overflow-x-auto min-w-0 flex-1">
        <button
          v-for="wt in worktrees.filter(w => !isDetached(w.id))"
          :key="wt.id"
          class="flex items-center gap-1.5 px-3 py-2 text-xs shrink-0 border-r border-[#313244] transition-colors"
          :class="
            viewMode === 'terminal' && wt.id === activeWorktreeId
              ? 'text-[#cba6f7]'
              : 'text-[#6c7086] hover:text-[#cdd6f4]'
          "
          @click="switchToWorktree(wt.id)"
        >
          <span>{{ wt.name }}</span>
          <span
            v-if="hotkeyChars.get(wt.id)"
            class="text-[9px] px-1 py-0.5 rounded font-mono font-medium shrink-0"
            style="background: rgba(203,166,247,0.15); color: #cba6f7; border: 1px solid rgba(203,166,247,0.3)"
          >{{ hotkeyChars.get(wt.id)!.toUpperCase() }}</span>
          <span
            v-if="notificationCounts.get(wt.id)"
            class="text-[9px] px-1 py-0.5 rounded-full font-bold shrink-0 leading-none"
            style="background: #f38ba8; color: #1e1e2e; min-width: 14px; text-align: center;"
          >{{ notificationCounts.get(wt.id) }}</span>
        </button>
      </div>

      <!-- ウィンドウコントロール -->
      <div class="flex shrink-0 items-stretch">
        <button
          class="flex items-center justify-center h-8 hover:bg-[#313244] text-[#6c7086] hover:text-[#cdd6f4] transition-colors"
          style="width: 42px;"
          :title="t('minimize')"
          @click="minimizeWindow"
        >
          <span class="pi pi-minus text-xs" />
        </button>
        <button
          class="flex items-center justify-center h-8 hover:bg-[#313244] text-[#6c7086] hover:text-[#cdd6f4] transition-colors"
          style="width: 42px;"
          :title="t('maximize')"
          @click="toggleMaximizeWindow"
        >
          <span class="pi pi-stop text-xs" />
        </button>
        <button
          class="flex items-center justify-center h-8 hover:bg-[#c0392b] hover:text-white text-[#6c7086] transition-colors"
          style="width: 42px;"
          :title="t('close')"
          @click="closeWindow"
        >
          <span class="pi pi-times text-xs" />
        </button>
      </div>
    </div>

    <!-- メインコンテンツ領域 -->
    <div ref="mainContentAreaRef" class="flex-1 min-h-0 relative">
      <!-- ホームビュー -->
      <HomeView
        ref="homeViewRef"
        v-show="viewMode === 'home'"
        class="absolute inset-0"
        :worktrees="worktrees"
        :thumbnail-urls="thumbnailUrls"
        :detached-worktrees="detachedWorktrees"
        :notifications="notificationCounts"
        :hotkey-chars="hotkeyChars"
        :loading-worktrees="loadingWorktrees"
        :auto-approvals="autoApprovalMap"
        :ai-judging-worktrees="aiJudgingWorktrees"
        @select-terminal="switchToTerminal"
        @add-worktree="showAddDialog = true"
        @remove-worktree="onRemoveWorktree"
        @add-terminal="onAddTerminal"
        @open-in-ide="onOpenInIde"
        @open-artifacts="onOpenArtifacts"
        @move-to-sub-window="onMoveToSubWindow"
        @move-to-main-window="onMoveToMainWindow"
        @focus-sub-window="onFocusSubWindow"
        @focus-all-sub-windows="onFocusAllSubWindows"
        @set-hotkey-char="onSetHotkeyChar"
        @toggle-auto-approval="onToggleAutoApproval"
        @cancel-ai-judging="onCancelAiJudging"
        @reorder-worktrees="reorderWorktree"
        @commit-reorder="saveWorktreeOrder"
        @cancel-reorder="restoreWorktreeOrder"
        @add-task="showAddTaskDialog = true"
        @remove-task="removeTask"
        @rerun-task="rerunTaskId = $event"
      />

      <!-- 設定ビュー -->
      <SettingsView
        v-show="viewMode === 'settings'"
        class="absolute inset-0"
      />

      <!-- ワークツリーごとのフレームレイアウト (detached は除外) -->
      <template v-for="wt in worktrees" :key="wt.id">
        <div
          v-if="!isDetached(wt.id)"
          v-show="viewMode === 'terminal' && wt.id === activeWorktreeId"
          class="absolute inset-0 flex flex-col"
        >
          <!-- ワークツリーヘッダー -->
          <WorktreeHeader
            :worktree-name="wt.name"
            :branch-name="wt.branchName"
            :hotkey-char="hotkeyChars.get(wt.id)"
            :auto-approval="autoApprovalMap.get(wt.id) ?? false"
            :ai-judging="aiJudgingWorktrees.has(wt.id)"
            :is-window-focused="isWindowFocused"
            :task-tooltip="getWorktreeTaskTooltip(wt.repositoryName, wt.branchName)"
            @open-in-ide="onOpenInIde(wt.id)"
            @open-artifacts="onOpenArtifacts(wt.id)"
            @cancel-ai-judging="onCancelAiJudging(wt.id)"
            @click-auto-approval="onClickAutoApproval(wt.id)"
          />
          <!-- フレームコンテンツ -->
          <div :data-frame-area="wt.id" class="flex-1 min-h-0 overflow-hidden">
            <FrameContainer
              v-if="worktreeFrameBundles.has(wt.id)"
              :node="worktreeFrameBundles.get(wt.id)!.frame.root.value"
              :terminal-entries="worktreeFrameBundles.get(wt.id)!.terminalEntries"
              :terminal-exit-codes="terminalExitCodes"
              :terminal-agent-status="terminalAgentStatus"
              :terminal-web-sessions="terminalWebSessions"
              @switch-terminal="(leafId, tid) => onFrameSwitch(wt.id, leafId, tid)"
              @close-terminal="(leafId, tid) => onFrameClose(wt.id, leafId, tid)"
              @tab-drop="(sl, tid, tl, idx) => onFrameTabDrop(wt.id, sl, tid, tl, idx)"
              @tab-edge-drop="(sl, tid, tl, dir) => onFrameTabEdgeDrop(wt.id, sl, tid, tl, dir)"
              @tab-reorder="(lid, tid, idx) => onFrameTabReorder(wt.id, lid, tid, idx)"
              @request-add-terminal="(leafId) => onFrameAddTerminal(wt.id, leafId)"
              @resize-end="() => {}"
            />
          </div>
        </div>
      </template>

      <!-- オフスクリーン TerminalView 群（DOM reparenting用） -->
      <div
        data-offscreen
        style="position:fixed; left:-10000px; top:-10000px; width:1000px; height:1000px; overflow:hidden; pointer-events:none"
      >
        <template v-for="wt in worktrees" :key="'off-' + wt.id">
          <template v-if="!isDetached(wt.id)">
            <TerminalView
              v-for="terminal in wt.terminals"
              :key="terminal.id"
              :ref="(el) => { const b = worktreeFrameBundles.get(wt.id); if (b) b.frame.setTerminalRef(terminal.id, el); }"
              :auto-start="!pendingManualStart.has(terminal.id) && !pendingSessionAttach.has(terminal.id)"
              :cwd="wt.path"
              :shell="resolveShell(wt.id)"
              :restore-snapshot="pendingSnapshots.get(terminal.id)"
              :initial-session-id="pendingSessionAttach.get(terminal.id)?.sessionId"
              :initial-snapshot="pendingSessionAttach.get(terminal.id)?.snapshot"
              @exit="onSessionExit(wt.id, terminal.id)"
              @ready="onTerminalReady(wt.id, terminal.id)"
              @title-change="onTerminalTitleChange(wt.id, terminal.id, $event)"
              @exit-code-change="terminalExitCodes.set(terminal.id, $event)"
            />
          </template>
        </template>
      </div>
    </div>

    <!-- タスク追加/再実行ダイアログ -->
    <AddTaskDialog
      v-if="showAddTaskDialog || rerunTaskId"
      :initial-prompt="rerunTaskId ? rerunPrompt : ''"
      :mode="rerunTaskId ? 'rerun' : 'add'"
      :show-remote-exec="(settings.aiAgent?.taskAddAgent ?? settings.aiAgent?.approvalAgent) === 'claudeCode'"
      :initial-remote-exec="settings.aiAgent?.remoteExec ?? false"
      @confirm="(prompt, remoteExec) => onAddTaskConfirm(prompt, remoteExec)"
      @cancel="onAddTaskCancel"
    />

    <!-- ワークツリー追加ダイアログ -->
    <AddWorktreeDialog
      v-if="showAddDialog"
      :repositories="settings.repositories"
      :worktree-base-dir="settings.worktreeBaseDir"
      :submitting="false"
      @confirm="(entry, sourceBranch) => onAddWorktreeConfirm(entry, sourceBranch)"
      @cancel="showAddDialog = false"
    />

    <!-- ワークツリー削除ダイアログ -->
    <RemoveWorktreeDialog
      v-if="showRemoveDialog && removeTargetWorktree"
      :worktree-name="removeTargetWorktree.name"
      :branch-name="removeTargetWorktree.branchName"
      :branches="removeBranches"
      :dirty-files="removeDirtyFiles"
      @confirm="onRemoveWorktreeConfirm"
      @archive="onArchiveWorktreeConfirm"
      @cancel="onRemoveWorktreeDismiss"
    />

    <!-- IDE 選択ダイアログ -->
    <IdeSelectDialog
      v-if="showIdeDialog"
      :ides="detectedIdes"
      @select="onIdeSelected"
      @cancel="showIdeDialog = false"
    />

    <!-- ホットキー文字割り当てダイアログ -->
    <HotkeyCharDialog
      v-if="showHotkeyCharDialog"
      :worktree-id="hotkeyCharTargetId"
      :worktree-name="worktrees.find((w) => w.id === hotkeyCharTargetId)?.name ?? ''"
      :current-char="hotkeyChars.get(hotkeyCharTargetId)"
      :used-chars="usedHotkeyChars"
      @confirm="onHotkeyCharConfirm"
      @clear="onHotkeyCharClear"
      @cancel="showHotkeyCharDialog = false"
    />

    <!-- 自動承認 追加プロンプト編集ダイアログ -->
    <AutoApprovalPromptDialog
      v-if="showAutoApprovalPromptDialog"
      :worktree-id="autoApprovalPromptTargetId"
      :worktree-name="worktrees.find((w) => w.id === autoApprovalPromptTargetId)?.name ?? ''"
      :current-prompt="autoApprovalPromptMap.get(autoApprovalPromptTargetId) ?? ''"
      :last-command="lastJudgedCommandMap.get(autoApprovalPromptTargetId) ?? ''"
      @save="onSaveAutoApprovalPrompt"
      @cancel="showAutoApprovalPromptDialog = false"
    />

    <!-- 通知トレイボタン (ワークツリー表示中は非表示) -->
    <TrayButton
      v-if="viewMode !== 'terminal'"
      :total-count="getTotalNotificationCount()"
      @click="onTrayButtonClick"
    />

    <!-- タスク進捗トースト -->
    <Toast position="bottom-right">
      <template #message="slotProps">
        <div class="toast-message-content">
          <i v-if="slotProps.message.severity === 'info'" class="pi pi-spinner pi-spin toast-spinner" />
          <div>
            <div class="font-semibold">{{ slotProps.message.summary }}</div>
            <div class="text-sm">{{ slotProps.message.detail }}</div>
          </div>
        </div>
      </template>
    </Toast>

    <!-- シャットダウン待機オーバーレイ -->
    <div v-if="isWaitingForShutdown" class="shutdown-overlay">
      <i class="pi pi-spinner pi-spin" style="font-size: 2.5rem; color: #cba6f7;" />
      <p style="color: #cdd6f4; margin-top: 16px;">{{ t('shuttingDown') }}</p>
    </div>
  </div>
</template>

<i18n lang="json">
{
  "en": {
    "taskAddSummary": "Add Task",
    "taskAddDetail": "Generating code...",
    "taskExecutingSummary": "Executing Task",
    "taskExecutingStartDetail": "Starting step execution ({count} steps)",
    "taskStepDetail": "Step {current}/{total}: {label}",
    "taskStepAddWorktree": "Adding worktree",
    "taskStepAgent": "Launching agent",
    "taskCompletedSummary": "Task Completed",
    "taskCompletedDetail": "All steps completed",
    "taskFailedSummary": "Task Failed",
    "deletingText": "Deleting...",
    "archivingText": "Archiving...",
    "creatingText": "Creating...",
    "deleteFailed": "Delete failed: {error}",
    "ideNotInstalled": "None of Cursor, VS Code, Antigravity are installed.",
    "ideNotInstalledTitle": "IDE not found",
    "ideLaunchFailed": "Failed to launch IDE: {error}",
    "lfsWarning": "Failed to fetch Git LFS files; worktree was created without LFS files.\nIf you need LFS files, run git lfs pull.",
    "worktreeCreateFailed": "Failed to create worktree: {error}",
    "copyTargetsFailed": "Some files could not be copied after worktree creation: {error}",
    "claudeHooksFailed": "Failed to write Claude Code notification hooks: {error}",
    "shuttingDown": "Shutting down...",
    "minimize": "Minimize",
    "maximize": "Maximize",
    "close": "Close"
  },
  "ja": {
    "taskAddSummary": "タスク追加",
    "taskAddDetail": "コード生成中...",
    "taskExecutingSummary": "タスク実行中",
    "taskExecutingStartDetail": "ステップ実行開始 ({count}件)",
    "taskStepDetail": "ステップ {current}/{total}: {label}",
    "taskStepAddWorktree": "ワークツリー追加中",
    "taskStepAgent": "エージェント起動中",
    "taskCompletedSummary": "タスク完了",
    "taskCompletedDetail": "すべてのステップが完了しました",
    "taskFailedSummary": "タスク失敗",
    "deletingText": "削除中...",
    "archivingText": "アーカイブ中...",
    "creatingText": "作成中...",
    "deleteFailed": "削除に失敗しました: {error}",
    "ideNotInstalled": "Cursor、VS Code、Antigravity のいずれもインストールされていません。",
    "ideNotInstalledTitle": "IDE が見つかりません",
    "ideLaunchFailed": "IDE の起動に失敗しました: {error}",
    "lfsWarning": "Git LFS のファイル取得に失敗したため、LFS ファイルをスキップしてワークツリーを作成しました。\nLFS ファイルが必要な場合は git lfs pull を実行してください。",
    "worktreeCreateFailed": "ワークツリーの作成に失敗しました: {error}",
    "copyTargetsFailed": "ワークツリー追加後のファイルコピーに失敗しました: {error}",
    "claudeHooksFailed": "Claude Code通知フックの書き込みに失敗しました: {error}",
    "shuttingDown": "終了しています...",
    "minimize": "最小化",
    "maximize": "最大化",
    "close": "閉じる"
  }
}
</i18n>
