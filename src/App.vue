<script setup lang="ts">
import { ref, reactive, nextTick, onMounted, computed, watch } from "vue";
import { renderToDataUrl } from "./composables/useTerminalThumbnail";
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
import TrayButton from "./components/TrayButton.vue";
import { message } from "@tauri-apps/plugin-dialog";
import { invoke } from "@tauri-apps/api/core";
import { useIdeSelect } from "./composables/useIdeSelect";
import { useSettings } from "./composables/useSettings";
import { useWorktrees } from "./composables/useWorktrees";
import { useI18n } from "vue-i18n";
import { useSubWindows } from "./composables/useSubWindows";
import { useCodeReviewWindow } from "./composables/useCodeReviewWindow";
import { useNotifications, sendOsNotification } from "./composables/useNotifications";
import { useTrayPopup } from "./composables/useTrayPopup";
import { useWindowFocus } from "./composables/useWindowFocus";
import { useTasks } from "./composables/useTasks";
import type { TrayWorktreeData, TrayTerminalData } from "./composables/useTrayPopup";
import type { WorktreeEntry, Repository } from "./types/settings";
import type { AddWorktreeTaskCode, AgentWorktreeTaskCode } from "./types/task";
import { useAddTaskDialog } from "./composables/useAddTaskDialog";
import type { FrameNode } from "./types/frame";
import type { SubTerminalEntry } from "./types/terminal";
import { useWorktreeFrameBundles } from "./composables/useWorktreeFrameBundles";
import { useHotkeyListener, bindingToAccelerator, matchesHotkey } from "./composables/useHotkeys";
import { useCodeReviewChatListener } from "./composables/useCodeReviewLineChat";
import { register, unregister } from "@tauri-apps/plugin-global-shortcut";
import { saveWindowState, StateFlags } from "@tauri-apps/plugin-window-state";
import { getRecentLines, analyzeForApproval, hasApprovalPrompt, cancelApproval } from "./utils/autoApproval";
import { useAutoHotkey } from "./composables/useAutoHotkey";
import { useShutdownGuard } from "./composables/useShutdownGuard";
import { debug } from "@tauri-apps/plugin-log";
import { platform } from "@tauri-apps/plugin-os";
import Toast from "primevue/toast";

const { t } = useI18n();

// ウィンドウのフォーカス状態
const { isWindowFocused } = useWindowFocus();

// 自動承認: ワークツリー ID → 有効/無効
const autoApprovalMap = reactive(new Map<string, boolean>());

// AI判定進行中のワークツリー ID セット
const aiJudgingWorktrees = reactive(new Set<string>());

// サブウィンドウフォーカス状態: ワークツリー ID → フォーカス中か
const subWindowFocusMap = reactive(new Map<string, boolean>());

type ViewMode = "home" | "settings" | "terminal";

const { settings, loadSettings, scheduleSave } = useSettings();
const { worktrees, loadWorktreesFromSettings, addWorktreePlaceholder, invokeWorktreeAdd, commitWorktree, rollbackWorktree, removeWorktree, listBranches, addTerminal, removeTerminal, updateTerminalTitle, saveTerminalSession, loadTerminalSession } = useWorktrees();
const { detachedWorktrees, isDetached, moveToSubWindow, moveToMainWindow, focusSubWindow, unregisterSubWindow, getPendingInitData, clearPendingInitData, getDetachedSessionId, registerTerminalSession, closeAllSubWindows } = useSubWindows();
const { notifications, initNotificationListener, addNotification, clearNotification, getNotifiedWorktreeIds, getTotalNotificationCount } = useNotifications();
const { openTrayPopup, closeTrayPopup, getPendingWorktrees, clearPendingWorktrees, setCurrentTrayWorktreeId, isTrayShowingWorktree, focusTrayWindow } = useTrayPopup();
const { closeAllCodeReviewWindows } = useCodeReviewWindow();
const { tryAutoAssignHotkey } = useAutoHotkey();
const { sortedTasks, removeTask } = useTasks();
const { showAddTaskDialog, rerunTaskId, rerunPrompt, onAddTaskConfirm, onAddTaskCancel } =
  useAddTaskDialog(async (code) => {
    if (code.type === "add_worktree") {
      await executeAddWorktree(code);
    } else if (code.type === "agent_worktree") {
      await executeAgentWorktree(code);
    }
  });

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

// ワークツリー削除ダイアログ
const showRemoveDialog = ref(false);
const removeTargetWorktree = ref<{ id: string; name: string; branchName: string } | null>(null);
const removeBranches = ref<string[]>([]);
const removeDirtyFiles = ref<{ path: string; status: string; staged: boolean }[]>([]);

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

function resolveShell(_worktreeId: string): string | undefined {
  return settings.value.terminal.shell || undefined;
}

function buildScriptCommand(repo: Repository, entry: WorktreeEntry): string {
  const scriptPath = repo.execScript!;
  const shell = resolveShell(entry.id);
  const repoName = repo.name;
  const wtName = entry.name;
  const shellLower = (shell ?? '').toLowerCase();

  // pty_manager.rs と同じロジック: 未指定時は Windows→powershell.exe、それ以外→SHELL
  const isWindows = platform() === "windows";
  const isPowerShell = shellLower.includes('powershell') || shellLower.includes('pwsh') || (shell === undefined && isWindows);
  const isCmd = !isPowerShell && shellLower.includes('cmd');

  if (isCmd) {
    return `set ORETACHI_REPO_NAME=${repoName}&& set ORETACHI_WORKTREE_NAME=${wtName}&& call "${scriptPath}"\r`;
  } else if (isPowerShell) {
    return `$env:ORETACHI_REPO_NAME="${repoName}"; $env:ORETACHI_WORKTREE_NAME="${wtName}"; Set-ExecutionPolicy -Scope Process Bypass; & "${scriptPath}"\r`;
  } else {
    return `ORETACHI_REPO_NAME="${repoName}" ORETACHI_WORKTREE_NAME="${wtName}" sh "${scriptPath}"\r`;
  }
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

async function onRemoveWorktree(worktreeId: string) {
  clearNotification(worktreeId);
  const worktree = worktrees.value.find((w) => w.id === worktreeId);
  if (!worktree) return;

  // ブランチ一覧 & ステータスを並行取得してダイアログ表示
  const [branches, dirtyFiles] = await Promise.all([
    listBranches(worktree.repositoryId).then((all) => all.filter((b) => b !== worktree.branchName)).catch(() => [] as string[]),
    invoke<{ path: string; status: string; staged: boolean }[]>("git_get_status", { repoPath: worktree.path }).catch(() => [] as { path: string; status: string; staged: boolean }[]),
  ]);

  removeTargetWorktree.value = { id: worktree.id, name: worktree.name, branchName: worktree.branchName };
  removeBranches.value = branches;
  removeDirtyFiles.value = dirtyFiles;
  showRemoveDialog.value = true;
}

async function onRemoveWorktreeConfirm(options: { mergeTo: string; deleteBranch: boolean; forceBranch: boolean }) {
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
  loadingWorktrees.set(worktreeId, t("deletingText"));
  try {
    // detached の場合はサブウィンドウを閉じる
    if (isDetached(worktreeId)) {
      await moveToMainWindow(worktreeId);
      subWindowFocusMap.delete(worktreeId);
    }

    // AI判定プロセスをキャンセル
    await cancelApproval(worktreeId);

    // 内部ターミナルを全て kill
    const bundle = worktreeFrameBundles.get(worktreeId);
    for (const terminal of [...worktree.terminals]) {
      const term = bundle?.terminalRefs.get(terminal.id) ?? getTerminalRef(terminal.id);
      if (term?.isRunning) {
        await term.kill();
      }
      terminalWorktreeMap.delete(terminal.id);
    }
    worktreeFrameBundles.delete(worktreeId);

    if (activeWorktreeId.value === worktreeId) {
      goHome();
    }

    try {
      await removeWorktree(worktreeId, {
        mergeTo: options.mergeTo || undefined,
        deleteBranch: options.deleteBranch,
        forceBranch: options.forceBranch,
      });
    } catch (e) {
      await message(t("deleteFailed", { error: e }), { kind: "error" });
    }
  } finally {
    loadingWorktrees.delete(worktreeId);
  }
}

async function onOpenInIde(worktreeId: string) {
  const worktree = worktrees.value.find((w) => w.id === worktreeId);
  if (!worktree) return;
  await openInIde(worktree.path, { worktreeId: worktree.id, worktreeName: worktree.name });
}

async function onAddWorktreeConfirm(entry: WorktreeEntry) {
  // ダイアログを即閉じ、一覧に仮エントリを表示
  showAddDialog.value = false;
  addWorktreePlaceholder(entry);
  loadingWorktrees.set(entry.id, t("creatingText"));

  try {
    const lfsSkipped = await invokeWorktreeAdd(entry);

    // 成功時: 設定に永続化
    commitWorktree(entry);
    tryAutoAssignHotkey(entry.id);

    // デフォルト: 自動承認
    if (settings.value.worktreeDefaults?.autoApproval) {
      autoApprovalMap.set(entry.id, true);
      const wtEntry = settings.value.worktrees.find((w) => w.id === entry.id);
      if (wtEntry) wtEntry.autoApproval = true;
    }

    // スクリプトがあればターミナルで実行するためにペンディング登録
    const repo = settings.value.repositories.find((r) => r.id === entry.repositoryId);
    if (repo?.execScript) {
      pendingScripts.set(entry.id, buildScriptCommand(repo, entry));
    }

    // ワークツリー作成後、自動でターミナルを1つ追加
    await onAddTerminal(entry.id);

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

// ─── タスク実行 ───────────────────────────────────────────────────────────────

function randomSuffix(): string {
  return Math.random().toString(36).slice(2, 6);
}

async function waitForSessionReady(worktreeId: string): Promise<number | null> {
  for (let i = 0; i < 100; i++) {
    const wt = worktrees.value.find((w) => w.id === worktreeId);
    const t = wt?.terminals[0];
    if (t) {
      const ref = getTerminalRef(t.id);
      const s = ref?.sessionId;
      if (s !== null && s !== undefined) return s;
    }
    await new Promise((r) => setTimeout(r, 100));
  }
  return null;
}

async function waitForScriptCompletion(worktreeId: string, timeoutMs = 300_000): Promise<void> {
  const sid = await waitForSessionReady(worktreeId);
  if (sid === null) return;

  const targetSid = sid;
  await new Promise<void>((resolve) => {
    let timer: ReturnType<typeof setTimeout>;
    let unlistenFn: (() => void) | null = null;

    timer = setTimeout(() => {
      unlistenFn?.();
      resolve();
    }, timeoutMs);

    listen<{ sessionId: number; data: number[] }>("pty-output", (event) => {
      if (event.payload.sessionId !== targetSid) return;
      const text = new TextDecoder().decode(new Uint8Array(event.payload.data));
      if (/\x1b\]777;exit_code;\d+/.test(text)) {
        clearTimeout(timer);
        unlistenFn?.();
        resolve();
      }
    }).then((fn) => {
      unlistenFn = fn;
    });
  });
}

async function executeAddWorktree(code: AddWorktreeTaskCode): Promise<string> {
  const repo = settings.value.repositories.find((r) => r.name === code.repository);
  if (!repo) throw new Error(`リポジトリが見つかりません: ${code.repository}`);

  const suffix = randomSuffix();
  const worktreeName = `${repo.name}-${suffix}`;
  const entry: WorktreeEntry = {
    id: `${Date.now()}-${suffix}`,
    name: worktreeName,
    repositoryId: repo.id,
    repositoryName: repo.name,
    path: `${settings.value.worktreeBaseDir}/${worktreeName}`,
    branchName: code.branch,
  };

  addWorktreePlaceholder(entry);
  loadingWorktrees.set(entry.id, t("creatingText"));

  try {
    await invokeWorktreeAdd(entry);
    commitWorktree(entry);
    tryAutoAssignHotkey(entry.id);

    // デフォルト: 自動承認
    if (settings.value.worktreeDefaults?.autoApproval) {
      autoApprovalMap.set(entry.id, true);
      const wtEntry = settings.value.worktrees.find((w) => w.id === entry.id);
      if (wtEntry) wtEntry.autoApproval = true;
    }

    if (repo.execScript) {
      pendingScripts.set(entry.id, buildScriptCommand(repo, entry));
    }

    await onAddTerminal(entry.id);
    await waitForSessionReady(entry.id);

    if (repo.execScript) {
      await waitForScriptCompletion(entry.id);
    }

    // デフォルト: サブウィンドウで開く
    if (settings.value.worktreeDefaults?.openInSubWindow) {
      await onMoveToSubWindow(entry.id);
    }
  } catch (e) {
    rollbackWorktree(entry.id);
    loadingWorktrees.delete(entry.id);
    throw e;
  }

  loadingWorktrees.delete(entry.id);
  return entry.id;
}

async function executeAgentWorktree(code: AgentWorktreeTaskCode): Promise<void> {
  // 既存のワークツリーを repository名 + branch名 で検索
  const wt = worktrees.value.find(
    (w) => w.repositoryName === code.repository && w.branchName === code.branch
  );
  if (!wt) {
    throw new Error(`ワークツリーが見つかりません: ${code.repository}/${code.branch}`);
  }

  // 初期ターミナル（execScript完了後のプロンプトが出ている状態）を再利用
  const terminal = wt.terminals[0];
  if (!terminal) {
    throw new Error(`ターミナルが見つかりません: ${wt.name}`);
  }

  // 一時ファイルにプロンプトを書き出し
  const tempPath = await invoke<string>("write_temp_prompt", { content: code.prompt });

  const agentKind = settings.value.aiAgent?.approvalAgent ?? "claudeCode";
  const isWindows = platform() === "windows";
  const shell = resolveShell(wt.id);
  const shellLower = (shell ?? "").toLowerCase();
  const isPowerShell =
    shellLower.includes("powershell") ||
    shellLower.includes("pwsh") ||
    (shell === undefined && isWindows);

  let agentCmd: string;
  switch (agentKind) {
    case "claudeCode": agentCmd = "claude --permission-mode plan"; break;
    case "geminiCli":  agentCmd = "gemini"; break;
    case "codexCli":   agentCmd = "codex"; break;
    case "clineCli":   agentCmd = "cline"; break;
    default:           agentCmd = agentKind;
  }

  let command: string;
  if (isPowerShell) {
    command = `$p = Get-Content "${tempPath}" -Raw -Encoding UTF8; ${agentCmd} $p; Remove-Item "${tempPath}"\r`;
  } else {
    command = `${agentCmd} "$(cat "${tempPath}")"; rm "${tempPath}"\r`;
  }

  if (isDetached(wt.id)) {
    // サブウィンドウに移動済みの場合: pty_write で直接PTYに書き込む
    const sid = getDetachedSessionId(terminal.id);
    if (sid === null) {
      throw new Error(`セッションIDが見つかりません: ${wt.name}`);
    }
    const bytes = Array.from(new TextEncoder().encode(command));
    await invoke("pty_write", { sessionId: sid, data: bytes });
    await invoke("pty_set_ai_agent", { sessionId: sid, isAgent: true });
  } else {
    // メインウィンドウ: terminalRef 経由で書き込む
    const termRef = getTerminalRef(terminal.id);
    if (!termRef) {
      throw new Error(`ターミナルが見つかりません: ${wt.name}`);
    }
    await termRef.waitForReady();
    await termRef.write(command);
    const sid = termRef.sessionId;
    if (sid != null) await invoke("pty_set_ai_agent", { sessionId: sid, isAgent: true });
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

  await moveToSubWindow(worktreeId, worktree.name, terminals, autoApprovalMap.get(worktreeId) ?? false, false, worktree.path, layout);

  // バンドルをクリーンアップ
  worktreeFrameBundles.delete(worktreeId);
}

function isWorktreeFocused(worktreeId: string): boolean {
  if (isDetached(worktreeId)) {
    return subWindowFocusMap.get(worktreeId) === true;
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

async function onToggleAutoApproval(worktreeId: string) {
  const current = autoApprovalMap.get(worktreeId) ?? false;
  autoApprovalMap.set(worktreeId, !current);

  // 設定ファイルに永続化
  const wtEntry = settings.value.worktrees.find((w) => w.id === worktreeId);
  if (wtEntry) {
    wtEntry.autoApproval = !current;
    scheduleSave();
  }

  // 自動承認を OFF にした時、AI判定中ならキャンセル
  if (current && aiJudgingWorktrees.has(worktreeId)) {
    await cancelApproval(worktreeId);
    if (isDetached(worktreeId)) {
      await emitTo(`sub-${worktreeId}`, "sub-cancel-auto-approve", {});
    }
  }

  if (isDetached(worktreeId)) {
    await emitTo(`sub-${worktreeId}`, "sub-set-auto-approval", { autoApproval: !current });
  }
}

async function onCancelAiJudging(worktreeId: string) {
  await cancelApproval(worktreeId);
  if (isDetached(worktreeId)) {
    await emitTo(`sub-${worktreeId}`, "sub-cancel-auto-approve", {});
  }
}

async function getSubWindowLayout(worktreeId: string): Promise<{ layout: FrameNode | null; terminals: SubTerminalEntry[] }> {
  return new Promise((resolve) => {
    const timeout = setTimeout(() => {
      unlisten();
      resolve({ layout: null, terminals: [] });
    }, 3000);

    let unlisten = () => {};
    listen<{ worktreeId: string; layout: FrameNode | null; terminals: SubTerminalEntry[]; windowSize?: unknown }>(
      "sub-layout-response",
      (event) => {
        if (event.payload.worktreeId === worktreeId) {
          clearTimeout(timeout);
          unlisten();
          resolve({ layout: event.payload.layout, terminals: event.payload.terminals ?? [] });
        }
      }
    ).then((fn) => { unlisten = fn; });

    emitTo(`sub-${worktreeId}`, "sub-get-layout", {}).catch(() => {
      clearTimeout(timeout);
      unlisten();
      resolve({ layout: null, terminals: [] });
    });
  });
}

async function onMoveToMainWindow(worktreeId: string) {
  // サブウィンドウからレイアウトとターミナル情報を取得（destroy前に）
  const { layout: savedLayout, terminals: savedTerminals } = isDetached(worktreeId)
    ? await getSubWindowLayout(worktreeId)
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

  subWindowFocusMap.delete(worktreeId);
}

async function onTrayButtonClick() {
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
      const layoutData = await new Promise<{ layout: FrameNode | null; terminals: TrayTerminalData[]; windowSize?: { width: number; height: number } } | null>((resolve) => {
        const timeout = setTimeout(() => {
          unlisten();
          resolve(null);
        }, 3000);

        let unlisten = () => {};
        listen<{ worktreeId: string; layout: FrameNode | null; terminals: TrayTerminalData[]; windowSize?: { width: number; height: number } }>(
          "sub-layout-response",
          (event) => {
            if (event.payload.worktreeId === worktreeId) {
              clearTimeout(timeout);
              unlisten();
              resolve({ layout: event.payload.layout, terminals: event.payload.terminals, windowSize: event.payload.windowSize });
            }
          }
        ).then((fn) => { unlisten = fn; });

        emitTo(`sub-${worktreeId}`, "sub-get-layout", {}).catch(() => {
          clearTimeout(timeout);
          unlisten();
          resolve(null);
        });
      });

      worktreeDataList.push({
        worktreeId,
        worktreeName: worktree.name,
        worktreePath: worktree.path,
        isDetached: true,
        layout: (layoutData?.layout ?? null) as import("./types/frame").FrameNode | null,
        terminals: layoutData?.terminals ?? [],
        windowSize: layoutData?.windowSize,
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

// メインウィンドウのホットキー登録
useHotkeyListener(() => {
  const hk = settings.value.hotkeys;
  if (!hk) return [];

  const actions = [];

  // terminalNext: アクティブワークツリーのフォーカスリーフで次のタブへ
  actions.push({
    binding: hk.terminalNext,
    handler: () => {
      if (!activeWorktreeId.value) return;
      const bundle = worktreeFrameBundles.get(activeWorktreeId.value);
      if (!bundle) return;
      const leafId = bundle.frame.lastFocusedLeafId.value;
      if (!leafId) return;
      const leaf = bundle.frame.getLeafsWithTerminals().find((l) => l.id === leafId);
      if (!leaf || leaf.terminalIds.length === 0) return;
      const idx = leaf.terminalIds.indexOf(leaf.activeTerminalId ?? -1);
      const nextIdx = idx === -1 ? 0 : (idx + 1) % leaf.terminalIds.length;
      bundle.frame.switchTerminal(leafId, leaf.terminalIds[nextIdx]);
    },
  });

  // terminalPrev: アクティブワークツリーのフォーカスリーフで前のタブへ
  actions.push({
    binding: hk.terminalPrev,
    handler: () => {
      if (!activeWorktreeId.value) return;
      const bundle = worktreeFrameBundles.get(activeWorktreeId.value);
      if (!bundle) return;
      const leafId = bundle.frame.lastFocusedLeafId.value;
      if (!leafId) return;
      const leaf = bundle.frame.getLeafsWithTerminals().find((l) => l.id === leafId);
      if (!leaf || leaf.terminalIds.length === 0) return;
      const idx = leaf.terminalIds.indexOf(leaf.activeTerminalId ?? -1);
      const prevIdx = idx <= 0 ? leaf.terminalIds.length - 1 : idx - 1;
      bundle.frame.switchTerminal(leafId, leaf.terminalIds[prevIdx]);
    },
  });

  // terminalAdd: アクティブワークツリーにターミナル追加
  actions.push({
    binding: hk.terminalAdd,
    handler: () => {
      const worktreeId = activeWorktreeId.value
        ?? worktrees.value.find((w) => !isDetached(w.id))?.id;
      if (worktreeId) onAddTerminal(worktreeId);
    },
  });

  // terminalClose: アクティブワークツリーのフォーカスリーフのアクティブターミナルを閉じる
  actions.push({
    binding: hk.terminalClose,
    handler: () => {
      if (viewMode.value !== "terminal" || !activeWorktreeId.value) return;
      const bundle = worktreeFrameBundles.get(activeWorktreeId.value);
      if (!bundle) return;
      const leafId = bundle.frame.lastFocusedLeafId.value;
      if (!leafId) return;
      const leaf = bundle.frame.getLeafsWithTerminals().find((l) => l.id === leafId);
      if (leaf?.activeTerminalId != null) {
        bundle.frame.closeTerminal(leafId, leaf.activeTerminalId);
      }
    },
  });

  // addTask: タスク追加ダイアログを開く
  if (hk.addTask) {
    actions.push({
      binding: hk.addTask,
      handler: () => {
        showAddTaskDialog.value = true;
      },
    });
  }

  // homeTab: ホームタブへ戻る
  if (hk.homeTab) {
    actions.push({
      binding: hk.homeTab,
      handler: () => {
        goHome();
      },
    });
  }

  // Alt+[char]: 対応するワークツリーにフォーカス
  // (matchesHotkey は使わず個別に処理するため空の binding で追加しない)
  // → 別途 keydown リスナーで対応

  return actions;
});

// Alt+[char] ワークツリーフォーカス用の共通ロジック
function focusWorktreeByChar(char: string) {
  const wt = worktrees.value.find((w) => {
    const entry = settings.value.worktrees.find((e) => e.id === w.id);
    return entry?.hotkeyChar === char;
  });
  if (!wt) return;

  if (isDetached(wt.id)) {
    focusSubWindow(wt.id);
  } else if (wt.terminals.length > 0) {
    switchToWorktree(wt.id);
  } else {
    onAddTerminal(wt.id);
  }
}

// Alt+[char] ワークツリーフォーカス用の keydown リスナー
function handleAltCharKey(event: KeyboardEvent) {
  if (event.type !== "keydown") return;
  if (event.isComposing || event.keyCode === 229) return;
  if (!event.altKey || event.ctrlKey || event.shiftKey) return;
  if (event.key.length !== 1) return;

  const char = event.key.toLowerCase();
  // homeTab ホットキーと重複する場合は Alt+[char] の処理をスキップ
  const homeTabBinding = settings.value.hotkeys?.homeTab;
  if (homeTabBinding && matchesHotkey(event, homeTabBinding)) return;

  const wt = worktrees.value.find((w) => {
    const entry = settings.value.worktrees.find((e) => e.id === w.id);
    return entry?.hotkeyChar === char;
  });
  if (!wt) return;

  event.preventDefault();
  event.stopPropagation();
  focusWorktreeByChar(char);
}

let globalShortcutRegistered = false;
let registeredAccelerator: string | null = null;

async function registerGlobalShortcut() {
  const binding = settings.value.hotkeys?.globalTrayPopup;
  if (!binding) return;
  const accelerator = bindingToAccelerator(binding);
  try {
    if (globalShortcutRegistered && registeredAccelerator) {
      await unregister(registeredAccelerator);
      globalShortcutRegistered = false;
      registeredAccelerator = null;
    }
    await register(accelerator, () => {
      onTrayButtonClick();
    });
    globalShortcutRegistered = true;
    registeredAccelerator = accelerator;
  } catch (e) {
    console.error("[GlobalShortcut] 登録失敗:", e);
  }
}

onMounted(async () => {
  await loadSettings();
  await getCurrentWindow().setAlwaysOnTop(settings.value.alwaysOnTop);
  loadWorktreesFromSettings();

  // 保存された自動承認状態を復元
  for (const wt of settings.value.worktrees) {
    if (wt.autoApproval === true) {
      autoApprovalMap.set(wt.id, true);
    }
  }

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

  // サブウィンドウ準備完了 → init データをイベントで送信（サブウィンドウ復元より前に登録必須）
  await listen<{ worktreeId: string }>("sub-ready", async (event) => {
    const { worktreeId } = event.payload;
    // 新しいサブウィンドウは作成時にフォーカスされている
    subWindowFocusMap.set(worktreeId, true);
    const initData = getPendingInitData(worktreeId);
    if (initData) {
      await emitTo(`sub-${worktreeId}`, "sub-init", {
        worktreeId,
        terminals: initData.terminals,
        autoApproval: initData.autoApproval,
        layout: initData.layout,
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

    await moveToSubWindow(wt.id, wt.name, subTerminals, autoApprovalMap.get(wt.id) ?? false, true, wt.path);
  }

  // Alt+char ホットキーリスナー登録
  window.addEventListener("keydown", handleAltCharKey, true);

  // 通知リスナー初期化 (ワークツリー名 → ID 解決関数と自動承認中は保留するコールバックを渡す)
  await initNotificationListener(
    (name: string) => worktrees.value.find((w) => w.name === name)?.id,
    (id: string) => autoApprovalMap.get(id) === true || isWorktreeFocused(id),
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
    }
  );

  // notify-worktree → 自動承認チェック
  await listen<{ worktree_name: string; kind: string }>("notify-worktree", async (event) => {
    const { worktree_name: worktreeName, kind } = event.payload;
    const wt = worktrees.value.find((w) => w.name === worktreeName);

    await debug(
      `[AutoApproval] notify-worktree received worktreeName=${worktreeName} resolved=${wt?.id ?? "null"} autoApproval=${wt ? autoApprovalMap.get(wt.id) : "undefined"}`
    );

    if (!wt) return;

    // 作業完了通知は承認チェック不要 → そのまま通知
    if (kind === "completed") {
      if (!isWorktreeFocused(wt.id)) {
        addNotification(wt.id, "completed");
        await sendOsNotification(wt.name, t("notification.titleCompleted"));
      }
      return;
    }

    if (!autoApprovalMap.get(wt.id)) return;

    // 重複防止: 既に同一ワークツリーのAI判定が進行中ならスキップ
    if (aiJudgingWorktrees.has(wt.id)) {
      await debug(`[AutoApproval] already in progress for ${wt.id}, skipping`);
      return;
    }

    if (isDetached(wt.id)) {
      await debug(`[AutoApproval] delegating to sub-window ${wt.id}`);
      await emitTo(`sub-${wt.id}`, "sub-try-auto-approve", {});
      return;
    }

    // ローカルターミナルのバッファを読み取り承認判定
    await debug(`[AutoApproval] local terminals check, count=${wt.terminals.length}`);
    aiJudgingWorktrees.add(wt.id);
    let approved = false;
    try {
      for (const t of wt.terminals) {
        const termRef = getTerminalRef(t.id);
        if (!termRef) { await debug(`[AutoApproval] tid=${t.id} termRef=null, skip`); continue; }
        const terminal = termRef.getTerminal();
        if (!terminal) { await debug(`[AutoApproval] tid=${t.id} terminal=null, skip`); continue; }
        const content = getRecentLines(terminal, 200);
        await debug(`[AutoApproval] tid=${t.id} content(last200)=${content.slice(-200)}`);
        if (await analyzeForApproval(wt.id, content, wt.path)) {
          // バッファ再チェック: AI判定完了後、承認プロンプトがまだあるか確認
          const freshContent = getRecentLines(terminal, 10);
          if (!hasApprovalPrompt(freshContent)) {
            await debug(`[AutoApproval] tid=${t.id} → prompt disappeared, skip Enter`);
            break;
          }
          await debug(`[AutoApproval] tid=${t.id} → approved, sending Enter`);
          await termRef.write("\r");
          approved = true;
          break;
        } else {
          await debug(`[AutoApproval] tid=${t.id} → not approved`);
        }
      }
    } finally {
      aiJudgingWorktrees.delete(wt.id);
    }
    if (!approved && !isWorktreeFocused(wt.id)) {
      await debug(`[AutoApproval] local: not approved → addNotification(${wt.id})`);
      addNotification(wt.id, "approval");
      await sendOsNotification(wt.name, t("notification.titleApproval"));
    }
  });

  // サブウィンドウからの自動承認結果 → 拒否時のみ通知
  await listen<{ worktreeId: string; approved: boolean }>("sub-auto-approve-result", async (event) => {
    const { worktreeId: wid, approved } = event.payload;
    await debug(`[AutoApproval] sub-auto-approve-result worktreeId=${wid} approved=${approved}`);
    if (!approved && !isWorktreeFocused(wid)) {
      addNotification(wid, "approval");
      const wtName = worktrees.value.find((w) => w.id === wid)?.name;
      if (wtName) await sendOsNotification(wtName, t("notification.titleApproval"));
    }
  });

  // ターミナル追加リクエスト (サブウィンドウから)
  await listen<{ worktreeId: string }>("sub-add-terminal-request", async (event) => {
    await handleSubAddTerminalRequest(event.payload.worktreeId);
  });

  // サブウィンドウ close 通知 (ユーザーが X ボタンで閉じた場合のみ)
  // 注: SubWindowApp は既に kill 済みのため、ここでは状態解除のみ
  await listen<{ worktreeId: string }>("sub-window-closing", async (event) => {
    const { worktreeId } = event.payload;

    subWindowFocusMap.delete(worktreeId);

    // detached 解除（ターミナルは SubWindowApp 側で既に kill 済み）
    unregisterSubWindow(worktreeId);

    // terminalWorktreeMap からエントリを削除
    const worktree = worktrees.value.find((w) => w.id === worktreeId);
    if (worktree) {
      for (const terminal of [...worktree.terminals]) {
        terminalWorktreeMap.delete(terminal.id);
      }
    }
  });

  // サブウィンドウのフォーカス状態変化を受信
  await listen<{ worktreeId: string; focused: boolean }>("sub-window-focus-changed", (event) => {
    subWindowFocusMap.set(event.payload.worktreeId, event.payload.focused);
  });

  // サブウィンドウでターミナルが削除された通知
  await listen<{ worktreeId: string; terminalId: number }>("sub-remove-terminal", (event) => {
    const { worktreeId, terminalId } = event.payload;
    removeTerminal(worktreeId, terminalId);
    terminalWorktreeMap.delete(terminalId);
    thumbnailUrls.delete(terminalId);
    terminalAgentStatus.delete(terminalId);
  });

  // サブウィンドウからのサムネイル受信
  await listen<{ terminalId: number; imageUrl: string }>(
    "sub-thumbnail-update",
    (event) => {
      thumbnailUrls.set(event.payload.terminalId, event.payload.imageUrl);
    }
  );

  // サブウィンドウからのタイトル変更通知
  await listen<{ worktreeId: string; terminalId: number; title: string }>(
    "sub-title-update",
    (event) => {
      const { worktreeId: wid, terminalId, title } = event.payload;
      updateTerminalTitle(wid, terminalId, title);
    }
  );

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

  // グローバルショートカット登録
  await registerGlobalShortcut();

  // 設定変更時にグローバルショートカット再登録 + always-on-top 反映
  await listen("settings-changed", async () => {
    await loadSettings();
    await getCurrentWindow().setAlwaysOnTop(settings.value.alwaysOnTop);
    await registerGlobalShortcut();
  });

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

  // サブウィンドウでのターミナルフォーカス時の通知クリア
  await listen<{ worktreeId: string }>("sub-clear-notification", (event) => {
    clearNotification(event.payload.worktreeId);
  });

  // サブウィンドウからの Alt+char ワークツリーフォーカス委譲
  await listen<{ char: string }>("sub-alt-char-focus", (event) => {
    focusWorktreeByChar(event.payload.char);
  });

  // トレイポップアップ閉鎖通知
  await listen("tray-closing", () => {
    closeTrayPopup();
  });

  // Code Review チャットボタン → AIエージェントターミナルへ書き込み
  await setupCodeReviewChatListener();

  // 初期ターミナルの worktreeId マッピングを構築
  for (const wt of worktrees.value) {
    for (const t of wt.terminals) {
      terminalWorktreeMap.set(t.id, wt.id);
    }
  }

  // ローカルターミナル用サムネイル生成ループ（変化があった場合のみ更新）
  setInterval(() => {
    for (const [wid, bundle] of worktreeFrameBundles) {
      if (isDetached(wid)) continue;
      for (const [id, ref] of bundle.terminalRefs) {
        const terminal = ref.getTerminal();
        if (terminal) {
          const url = renderToDataUrl(terminal);
          if (url && url !== thumbnailUrls.get(id)) thumbnailUrls.set(id, url);
        }
      }
    }
  }, 1000);

  // メインウィンドウ閉じ時: 全サブウィンドウを閉じてからアプリ終了
  await getCurrentWindow().onCloseRequested(async (event) => {
    event.preventDefault();
    if (isWaitingForShutdown.value) return; // 二重クリック防止

    // ワークツリー操作/タスク実行中の場合は完了まで待機
    if (isBusyForShutdown.value) {
      isWaitingForShutdown.value = true;
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
    await closeAllCodeReviewWindows();
    await closeAllSubWindows();
    for (const [, bundle] of worktreeFrameBundles) {
      for (const [, term] of bundle.terminalRefs) {
        if (term?.isRunning) {
          await term.kill();
        }
      }
    }
    await getCurrentWindow().destroy();
  });
});
</script>

<template>
  <div class="h-screen flex flex-col bg-[#1e1e2e] text-[#cdd6f4] select-none">
    <!-- タブバー -->
    <div 
      class="flex items-center border-b shrink-0 min-h-0 transition-colors duration-200"
      :class="isWindowFocused ? 'bg-gradient-to-r from-[#181825] via-[#2a2a3f] to-[#181825] animate-gradient-x border-[#cba6f7]/50' : 'bg-[#11111b] opacity-90 border-[#313244]'"
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
              ? 'bg-[#1e1e2e] text-[#cba6f7]'
              : 'bg-[#181825] text-[#6c7086] hover:text-[#cdd6f4]'
          "
          @click="switchToWorktree(wt.id)"
        >
          <span>{{ wt.name }}</span>
          <span
            v-if="notificationCounts.get(wt.id)"
            class="text-[9px] px-1 py-0.5 rounded-full font-bold shrink-0 leading-none"
            style="background: #f38ba8; color: #1e1e2e; min-width: 14px; text-align: center;"
          >{{ notificationCounts.get(wt.id) }}</span>
        </button>
      </div>
    </div>

    <!-- メインコンテンツ領域 -->
    <div ref="mainContentAreaRef" class="flex-1 min-h-0 relative">
      <!-- ホームビュー -->
      <HomeView
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
        :tasks="sortedTasks"
        @select-terminal="switchToTerminal"
        @add-worktree="showAddDialog = true"
        @remove-worktree="onRemoveWorktree"
        @add-terminal="onAddTerminal"
        @open-in-ide="onOpenInIde"
        @move-to-sub-window="onMoveToSubWindow"
        @move-to-main-window="onMoveToMainWindow"
        @focus-sub-window="onFocusSubWindow"
        @focus-all-sub-windows="onFocusAllSubWindows"
        @set-hotkey-char="onSetHotkeyChar"
        @toggle-auto-approval="onToggleAutoApproval"
        @cancel-ai-judging="onCancelAiJudging"
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
            :hotkey-char="hotkeyChars.get(wt.id)"
            :auto-approval="autoApprovalMap.get(wt.id) ?? false"
            :ai-judging="aiJudgingWorktrees.has(wt.id)"
            :is-window-focused="isWindowFocused"
            @open-in-ide="onOpenInIde(wt.id)"
            @cancel-ai-judging="onCancelAiJudging(wt.id)"
          />
          <!-- フレームコンテンツ -->
          <div :data-frame-area="wt.id" class="flex-1 min-h-0 overflow-hidden">
            <FrameContainer
              v-if="worktreeFrameBundles.has(wt.id)"
              :node="worktreeFrameBundles.get(wt.id)!.frame.root.value"
              :terminal-entries="worktreeFrameBundles.get(wt.id)!.terminalEntries"
              :terminal-exit-codes="terminalExitCodes"
              :terminal-agent-status="terminalAgentStatus"
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
      @confirm="onAddTaskConfirm"
      @cancel="onAddTaskCancel"
    />

    <!-- ワークツリー追加ダイアログ -->
    <AddWorktreeDialog
      v-if="showAddDialog"
      :repositories="settings.repositories"
      :worktree-base-dir="settings.worktreeBaseDir"
      :submitting="false"
      @confirm="onAddWorktreeConfirm"
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
      @cancel="showRemoveDialog = false; removeDirtyFiles = []"
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
    "creatingText": "Creating...",
    "deleteFailed": "Delete failed: {error}",
    "ideNotInstalled": "None of Cursor, VS Code, Antigravity are installed.",
    "ideNotInstalledTitle": "IDE not found",
    "ideLaunchFailed": "Failed to launch IDE: {error}",
    "lfsWarning": "Failed to fetch Git LFS files; worktree was created without LFS files.\nIf you need LFS files, run git lfs pull.",
    "worktreeCreateFailed": "Failed to create worktree: {error}",
    "shuttingDown": "Waiting for operations to finish..."
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
    "creatingText": "作成中...",
    "deleteFailed": "削除に失敗しました: {error}",
    "ideNotInstalled": "Cursor、VS Code、Antigravity のいずれもインストールされていません。",
    "ideNotInstalledTitle": "IDE が見つかりません",
    "ideLaunchFailed": "IDE の起動に失敗しました: {error}",
    "lfsWarning": "Git LFS のファイル取得に失敗したため、LFS ファイルをスキップしてワークツリーを作成しました。\nLFS ファイルが必要な場合は git lfs pull を実行してください。",
    "worktreeCreateFailed": "ワークツリーの作成に失敗しました: {error}",
    "shuttingDown": "処理の完了を待っています..."
  }
}
</i18n>
