<script setup lang="ts">
import { ref, reactive, nextTick, onMounted, markRaw, computed } from "vue";
import { renderToDataUrl } from "./composables/useTerminalThumbnail";
import { listen, emitTo } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import TerminalView from "./components/TerminalView.vue";
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
import type { AddWorktreeTaskCode, AgentWorktreeTaskCode, TaskProcessCode } from "./types/task";
import type { FrameNode } from "./types/frame";
import { useHotkeyListener, bindingToAccelerator } from "./composables/useHotkeys";
import { register, unregister } from "@tauri-apps/plugin-global-shortcut";
import { saveWindowState, StateFlags } from "@tauri-apps/plugin-window-state";
import { getRecentLines, analyzeForApproval, hasApprovalPrompt, cancelApproval } from "./utils/autoApproval";
import { useAutoHotkey } from "./composables/useAutoHotkey";
import { debug } from "@tauri-apps/plugin-log";
import { platform } from "@tauri-apps/plugin-os";
import Toast from "primevue/toast";
import type { ToastMessageOptions } from "primevue/toast";
import { useToast } from "primevue/usetoast";

const toast = useToast();
let activeTaskToast: ToastMessageOptions | null = null;

function showTaskToast(options: ToastMessageOptions): void {
  if (activeTaskToast) {
    toast.remove(activeTaskToast);
    activeTaskToast = null;
  }
  if (options.life === undefined) {
    // 永続表示のメッセージは追跡する
    activeTaskToast = options;
  }
  toast.add(options);
}

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
const { openTrayPopup, closeTrayPopup, getPendingWorktrees, clearPendingWorktrees } = useTrayPopup();
const { closeAllCodeReviewWindows } = useCodeReviewWindow();
const { tryAutoAssignHotkey } = useAutoHotkey();
const { sortedTasks, addTask, setTaskSteps, updateStepStatus, updateTaskStatus, removeTask } = useTasks();
const showAddTaskDialog = ref(false);

// HomeView / WorktreeCard 向け: Map<string, number> 形式を維持
const notificationCounts = computed(() => {
  const map = new Map<string, number>();
  for (const [id, entry] of notifications) map.set(id, entry.count);
  return map;
});

const viewMode = ref<ViewMode>("home");

// アクティブなターミナルの識別情報
const activeTerminalId = ref<number | null>(null);

// terminalId → TerminalView インスタンス
const terminalRefs = reactive(new Map<number, InstanceType<typeof TerminalView>>());

// terminalId → 直近コマンドの終了コード (null = 未実行)
const terminalExitCodes = reactive(new Map<number, number>());

// terminalId → AIエージェント稼働中フラグ
const terminalAgentStatus = reactive(new Map<number, boolean>());

// terminalId → サムネイル data URL
const thumbnailUrls = reactive(new Map<number, string>());

// terminalId → worktreeId のマッピング（ターミナルがどのワークツリーに属するか）
const terminalWorktreeMap = new Map<number, string>();

// worktreeId → 実行待ちスクリプトコマンド文字列
const pendingScripts = new Map<string, string>();

// terminalId → 復元スナップショット（起動時セッション復元用）
const pendingSnapshots = new Map<number, string>();

// ワークツリー追加ダイアログ
const showAddDialog = ref(false);


// 削除中のワークツリー ID セット
const loadingWorktrees = reactive(new Map<string, string>());

// ワークツリー削除ダイアログ
const showRemoveDialog = ref(false);
const removeTargetWorktree = ref<{ id: string; name: string; branchName: string } | null>(null);
const removeBranches = ref<string[]>([]);

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

function setTerminalRef(terminalId: number, el: unknown) {
  if (el) {
    terminalRefs.set(terminalId, markRaw(el as InstanceType<typeof TerminalView>));
  } else {
    terminalRefs.delete(terminalId);
  }
}

async function onTerminalReady(terminalId: number) {
  const ref = terminalRefs.get(terminalId);
  if (ref) {
    terminalRefs.delete(terminalId);
    terminalRefs.set(terminalId, ref);
  }
  pendingSnapshots.delete(terminalId);
  const worktreeId = terminalWorktreeMap.get(terminalId);
  if (worktreeId) {
    const command = pendingScripts.get(worktreeId);
    if (command) {
      pendingScripts.delete(worktreeId);
      await ref?.write(command);
    }
  }
}

// タブバー用: 全ターミナル一覧 (worktree名/terminal名) — detached ワークツリーは除外
interface TabEntry {
  terminalId: number;
  worktreeId: string;
  label: string;
  cwd: string;
}

function buildTabs(): TabEntry[] {
  const result: TabEntry[] = [];
  for (const wt of worktrees.value) {
    if (isDetached(wt.id)) continue;
    for (const t of wt.terminals) {
      result.push({
        terminalId: t.id,
        worktreeId: wt.id,
        label: `${wt.name}/${t.title}`,
        cwd: wt.path,
      });
    }
  }
  return result;
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

  viewMode.value = "terminal";
  activeTerminalId.value = terminalId;
  await nextTick();
  const term = terminalRefs.get(terminalId);
  if (term) {
    await term.handleTabActivated();
    term.focus();
  }
}

function goHome() {
  viewMode.value = "home";
  activeTerminalId.value = null;
}

function goSettings() {
  viewMode.value = "settings";
  activeTerminalId.value = null;
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
    return `$env:ORETACHI_REPO_NAME="${repoName}"; $env:ORETACHI_WORKTREE_NAME="${wtName}"; & "${scriptPath}"\r`;
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
  terminalWorktreeMap.set(terminal.id, worktreeId);

  // TerminalView マウント時に v-show=true になるよう、nextTick の前に viewMode と activeTerminalId をセット
  viewMode.value = "terminal";
  activeTerminalId.value = terminal.id;

  await nextTick();

  const term = terminalRefs.get(terminal.id);
  if (term) {
    await term.handleTabActivated();
    term.focus();
  }
}

function onTerminalExitCodeChange(terminalId: number, exitCode: number) {
  terminalExitCodes.set(terminalId, exitCode);
}

async function onRemoveTerminal(worktreeId: string, terminalId: number) {
  const term = terminalRefs.get(terminalId);
  if (term?.isRunning) {
    await term.kill();
  }
  terminalWorktreeMap.delete(terminalId);
  terminalExitCodes.delete(terminalId);
  terminalAgentStatus.delete(terminalId);
  removeTerminal(worktreeId, terminalId);

  // アクティブターミナルが削除された場合、ホームへ
  if (activeTerminalId.value === terminalId) {
    goHome();
  }
}

async function onTabClose(terminalId: number) {
  const worktreeId = terminalWorktreeMap.get(terminalId);
  if (worktreeId) {
    await onRemoveTerminal(worktreeId, terminalId);
  }
}

async function onSessionExit(terminalId: number) {
  await onTabClose(terminalId);
}

async function onRemoveWorktree(worktreeId: string) {
  clearNotification(worktreeId);
  const worktree = worktrees.value.find((w) => w.id === worktreeId);
  if (!worktree) return;

  // ブランチ一覧を取得してダイアログ表示
  let branches: string[] = [];
  try {
    const all = await listBranches(worktree.repositoryId);
    branches = all.filter((b) => b !== worktree.branchName);
  } catch {
    branches = [];
  }

  removeTargetWorktree.value = { id: worktree.id, name: worktree.name, branchName: worktree.branchName };
  removeBranches.value = branches;
  showRemoveDialog.value = true;
}

async function onRemoveWorktreeConfirm(options: { mergeTo: string; deleteBranch: boolean; forceBranch: boolean }) {
  if (!removeTargetWorktree.value) return;
  const { id: worktreeId } = removeTargetWorktree.value;

  const worktree = worktrees.value.find((w) => w.id === worktreeId);
  if (!worktree) {
    showRemoveDialog.value = false;
    return;
  }

  showRemoveDialog.value = false;
  removeTargetWorktree.value = null;
  removeBranches.value = [];
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
    for (const terminal of [...worktree.terminals]) {
      const term = terminalRefs.get(terminal.id);
      if (term?.isRunning) {
        await term.kill();
      }
      terminalWorktreeMap.delete(terminal.id);
    }

    if (activeTerminalId.value !== null) {
      const activeWorktreeId = terminalWorktreeMap.get(activeTerminalId.value);
      if (activeWorktreeId === worktreeId) {
        goHome();
      }
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

async function waitForScriptCompletion(worktreeId: string, timeoutMs = 300_000): Promise<void> {
  // sessionId が確定するまでポーリング
  let sid: number | null = null;
  for (let i = 0; i < 100; i++) {
    const wt = worktrees.value.find((w) => w.id === worktreeId);
    const t = wt?.terminals[0];
    if (t) {
      const ref = terminalRefs.get(t.id);
      const s = ref?.sessionId;
      if (s !== null && s !== undefined) {
        sid = s;
        break;
      }
    }
    await new Promise((r) => setTimeout(r, 100));
  }
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
    const termRef = terminalRefs.get(terminal.id);
    if (!termRef) {
      throw new Error(`ターミナルが見つかりません: ${wt.name}`);
    }
    await termRef.waitForReady();
    await termRef.write(command);
    const sid = termRef.sessionId;
    if (sid != null) await invoke("pty_set_ai_agent", { sessionId: sid, isAgent: true });
  }
}

async function executeTaskSteps(taskId: string): Promise<void> {
  const { tasks } = useTasks();
  const task = tasks.value.find((t) => t.id === taskId);
  if (!task) return;

  for (let i = 0; i < task.steps.length; i++) {
    const step = task.steps[i];
    updateStepStatus(taskId, i, "running");

    const stepLabel = step.code.type === "add_worktree" ? "ワークツリー追加中" : "エージェント起動中";
    showTaskToast({
      severity: "info",
      summary: "タスク実行中",
      detail: `ステップ ${i + 1}/${task.steps.length}: ${stepLabel}`,
    });

    try {
      if (step.code.type === "add_worktree") {
        await executeAddWorktree(step.code);
      } else if (step.code.type === "agent_worktree") {
        await executeAgentWorktree(step.code);
      }
      updateStepStatus(taskId, i, "done");
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      updateStepStatus(taskId, i, "error", msg);
      throw e;
    }
  }
}

async function onAddTaskConfirm(prompt: string): Promise<void> {
  showAddTaskDialog.value = false;
  const task = addTask(prompt);

  showTaskToast({
    severity: "info",
    summary: "タスク追加",
    detail: "コード生成中...",
  });

  try {
    const result = await invoke<string>("task_generate", { prompt });
    const taskProcessCode = JSON.parse(result) as TaskProcessCode;
    setTaskSteps(task.id, taskProcessCode.code);

    const stepCount = taskProcessCode.code.length;
    showTaskToast({
      severity: "info",
      summary: "タスク実行中",
      detail: `ステップ実行開始 (${stepCount}件)`,
    });

    await executeTaskSteps(task.id);
    updateTaskStatus(task.id, "completed");

    showTaskToast({
      severity: "success",
      summary: "タスク完了",
      detail: "すべてのステップが完了しました",
      life: 3000,
    });
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    updateTaskStatus(task.id, "error", msg);

    showTaskToast({
      severity: "error",
      summary: "タスク失敗",
      detail: msg,
      life: 5000,
    });
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

  // 各ターミナルの sessionId とスナップショットを収集
  const terminals = worktree.terminals.map((t) => {
    const termRef = terminalRefs.get(t.id);
    const sessionId = termRef?.sessionId ?? null;
    const snapshot = sessionId !== null ? (termRef?.serializeBuffer() ?? "") : "";
    return {
      id: t.id,
      title: t.title,
      sessionId: sessionId ?? 0,
      snapshot,
    };
  }).filter((t) => t.sessionId !== 0);


  // 各ターミナルの PTY を detach (アンマウント時に kill させない)

  for (const t of worktree.terminals) {
    const termRef = terminalRefs.get(t.id);
    termRef?.detach();
  }

  // アクティブターミナルがこのワークツリーに属する場合はホームへ
  if (activeTerminalId.value !== null) {
    const activeWtId = terminalWorktreeMap.get(activeTerminalId.value);
    if (activeWtId === worktreeId) {
      goHome();
    }
  }

  await moveToSubWindow(worktreeId, worktree.name, terminals, autoApprovalMap.get(worktreeId) ?? false, false, worktree.path);
}

function isWorktreeFocused(worktreeId: string): boolean {
  if (isDetached(worktreeId)) {
    return subWindowFocusMap.get(worktreeId) === true;
  }
  if (!isWindowFocused.value || viewMode.value !== "terminal") return false;
  const wt = worktrees.value.find((w) => w.id === worktreeId);
  return wt?.terminals.some((t) => t.id === activeTerminalId.value) ?? false;
}

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

function onTerminalTitleChange(terminalId: number, title: string) {
  const worktreeId = terminalWorktreeMap.get(terminalId);
  if (worktreeId) updateTerminalTitle(worktreeId, terminalId, title);
}

async function onMoveToMainWindow(worktreeId: string) {
  await moveToMainWindow(worktreeId);
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
        isDetached: true,
        layout: (layoutData?.layout ?? null) as import("./types/frame").FrameNode | null,
        terminals: layoutData?.terminals ?? [],
        windowSize: layoutData?.windowSize,
      });
    } else {
      // メインウィンドウのターミナル情報を収集
      const terminals: TrayTerminalData[] = worktree.terminals.map((t) => {
        const termRef = terminalRefs.get(t.id);
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

      worktreeDataList.push({
        worktreeId,
        worktreeName: worktree.name,
        isDetached: false,
        layout: null,
        terminals,
      });
    }
  }

  if (worktreeDataList.length === 0) return;

  await openTrayPopup(worktreeDataList);
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

  // terminalNext: 次のタブへ循環
  actions.push({
    binding: hk.terminalNext,
    handler: () => {
      const tabs = buildTabs();
      if (tabs.length === 0) return;
      const idx = tabs.findIndex((t) => t.terminalId === activeTerminalId.value);
      const nextIdx = idx === -1 ? 0 : (idx + 1) % tabs.length;
      switchToTerminal(tabs[nextIdx].terminalId);
    },
  });

  // terminalPrev: 前のタブへ循環
  actions.push({
    binding: hk.terminalPrev,
    handler: () => {
      const tabs = buildTabs();
      if (tabs.length === 0) return;
      const idx = tabs.findIndex((t) => t.terminalId === activeTerminalId.value);
      const prevIdx = idx <= 0 ? tabs.length - 1 : idx - 1;
      switchToTerminal(tabs[prevIdx].terminalId);
    },
  });

  // terminalAdd: アクティブワークツリーにターミナル追加
  actions.push({
    binding: hk.terminalAdd,
    handler: () => {
      if (viewMode.value === "home") return;
      let worktreeId: string | undefined;
      if (activeTerminalId.value !== null) {
        worktreeId = terminalWorktreeMap.get(activeTerminalId.value);
      }
      if (!worktreeId) {
        // 最初の非detachedワークツリーに追加
        const wt = worktrees.value.find((w) => !isDetached(w.id));
        worktreeId = wt?.id;
      }
      if (worktreeId) onAddTerminal(worktreeId);
    },
  });

  // terminalClose: アクティブターミナルを閉じる
  actions.push({
    binding: hk.terminalClose,
    handler: () => {
      if (viewMode.value !== "terminal" || activeTerminalId.value === null) return;
      onTabClose(activeTerminalId.value);
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
    switchToTerminal(wt.terminals[0].id);
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
  }

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
          switchToTerminal(wt.terminals[0].id);
        }
        getCurrentWindow().setFocus();
      }
    },
    t("notification.title")
  );

  // notify-worktree → 自動承認チェック
  await listen<string>("notify-worktree", async (event) => {
    const worktreeName = event.payload;
    const wt = worktrees.value.find((w) => w.name === worktreeName);

    await debug(
      `[AutoApproval] notify-worktree received worktreeName=${worktreeName} resolved=${wt?.id ?? "null"} autoApproval=${wt ? autoApprovalMap.get(wt.id) : "undefined"}`
    );

    if (!wt) return;
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
        const termRef = terminalRefs.get(t.id);
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
      addNotification(wt.id);
      await sendOsNotification(wt.name, t("notification.title"));
    }
  });

  // サブウィンドウからの自動承認結果 → 拒否時のみ通知
  await listen<{ worktreeId: string; approved: boolean }>("sub-auto-approve-result", async (event) => {
    const { worktreeId: wid, approved } = event.payload;
    await debug(`[AutoApproval] sub-auto-approve-result worktreeId=${wid} approved=${approved}`);
    if (!approved && !isWorktreeFocused(wid)) {
      addNotification(wid);
      const wtName = worktrees.value.find((w) => w.id === wid)?.name;
      if (wtName) await sendOsNotification(wtName, t("notification.title"));
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
      const { worktreeId, terminalId, title } = event.payload;
      updateTerminalTitle(worktreeId, terminalId, title);
    }
  );

  // AIエージェントインジケーター: pty-ai-agent-changed を受信して terminalAgentStatus を更新
  await listen<{ sessions: Record<number, boolean> }>("pty-ai-agent-changed", (event) => {
    // sessionId → terminalId の逆引きマップを構築
    const sessionToTerminal = new Map<number, number>();
    for (const [tid, termRef] of terminalRefs) {
      const sid = termRef?.sessionId;
      if (sid != null) sessionToTerminal.set(sid, tid);
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
      } catch (e) {
        console.error("tray-init 送信失敗:", e);
      }
      clearPendingWorktrees();
    }
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

  // 初期ターミナルの worktreeId マッピングを構築
  for (const wt of worktrees.value) {
    for (const t of wt.terminals) {
      terminalWorktreeMap.set(t.id, wt.id);
    }
  }

  // ローカルターミナル用サムネイル生成ループ（変化があった場合のみ更新）
  setInterval(() => {
    for (const [id, ref] of terminalRefs) {
      const wtId = terminalWorktreeMap.get(id);
      if (wtId && isDetached(wtId)) continue;
      const terminal = ref.getTerminal();
      if (terminal) {
        const url = renderToDataUrl(terminal);
        if (url && url !== thumbnailUrls.get(id)) thumbnailUrls.set(id, url);
      }
    }
  }, 1000);

  // メインウィンドウ閉じ時: 全サブウィンドウを閉じてからアプリ終了
  await getCurrentWindow().onCloseRequested(async (event) => {
    event.preventDefault();

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
        const terminals = wt.terminals.map((t) => {
          const termRef = terminalRefs.get(t.id);
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
    for (const [, term] of terminalRefs) {
      if (term?.isRunning) {
        await term.kill();
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

      <!-- ターミナルタブ一覧 -->
      <div class="flex overflow-x-auto min-w-0 flex-1">
        <button
          v-for="tab in buildTabs()"
          :key="tab.terminalId"
          class="group flex items-center gap-1 px-3 py-2 text-xs shrink-0 border-r border-[#313244] transition-colors"
          :class="
            viewMode === 'terminal' && tab.terminalId === activeTerminalId
              ? 'bg-[#1e1e2e] text-[#cba6f7]'
              : 'bg-[#181825] text-[#6c7086] hover:text-[#cdd6f4]'
          "
          @click="switchToTerminal(tab.terminalId)"
        >
          <span>{{ tab.label }}</span>
          <span
            v-if="terminalAgentStatus.get(tab.terminalId)"
            class="pi pi-microchip text-[10px] text-[#a6e3a1] shrink-0"
            title="AI Agent"
          />
          <span
            v-else-if="terminalExitCodes.has(tab.terminalId)"
            class="w-2 h-2 rounded-full inline-block shrink-0"
            :class="terminalExitCodes.get(tab.terminalId) === 0 ? 'bg-[#89b4fa]' : 'bg-[#f38ba8]'"
            :title="'Exit: ' + terminalExitCodes.get(tab.terminalId)"
          />
          <span
            class="pi pi-times text-[10px] opacity-0 group-hover:opacity-100 hover:text-[#f38ba8] transition-opacity ml-1"
            @click.stop="onTabClose(tab.terminalId)"
          />
        </button>
      </div>
    </div>

    <!-- メインコンテンツ領域 -->
    <div class="flex-1 min-h-0 relative">
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
      />

      <!-- 設定ビュー -->
      <SettingsView
        v-show="viewMode === 'settings'"
        class="absolute inset-0"
      />

      <!-- ターミナル群 (detached ワークツリーは DOM から除外) -->
      <template v-for="wt in worktrees" :key="wt.id">
        <template v-if="!isDetached(wt.id)">
          <div
            v-for="terminal in wt.terminals"
            :key="terminal.id"
            v-show="viewMode === 'terminal' && terminal.id === activeTerminalId"
            class="absolute inset-0"
          >
            <TerminalView
              :ref="(el) => setTerminalRef(terminal.id, el)"
              :auto-start="true"
              :cwd="wt.path"
              :shell="resolveShell(wt.id)"
              :restore-snapshot="pendingSnapshots.get(terminal.id)"
              @exit="onSessionExit(terminal.id)"
              @ready="onTerminalReady(terminal.id)"
              @title-change="onTerminalTitleChange(terminal.id, $event)"
              @exit-code-change="onTerminalExitCodeChange(terminal.id, $event)"
            />
          </div>
        </template>
      </template>
    </div>

    <!-- タスク追加ダイアログ -->
    <AddTaskDialog
      v-if="showAddTaskDialog"
      @confirm="onAddTaskConfirm"
      @cancel="showAddTaskDialog = false"
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
      @confirm="onRemoveWorktreeConfirm"
      @cancel="showRemoveDialog = false"
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

    <!-- 通知トレイボタン -->
    <TrayButton
      :total-count="getTotalNotificationCount()"
      @click="onTrayButtonClick"
    />

    <!-- タスク進捗トースト -->
    <Toast position="bottom-right" />
  </div>
</template>

<i18n lang="json">
{
  "en": {
    "deletingText": "Deleting...",
    "creatingText": "Creating...",
    "deleteFailed": "Delete failed: {error}",
    "ideNotInstalled": "None of Cursor, VS Code, Antigravity are installed.",
    "ideNotInstalledTitle": "IDE not found",
    "ideLaunchFailed": "Failed to launch IDE: {error}",
    "lfsWarning": "Failed to fetch Git LFS files; worktree was created without LFS files.\nIf you need LFS files, run git lfs pull.",
    "worktreeCreateFailed": "Failed to create worktree: {error}"
  },
  "ja": {
    "deletingText": "削除中...",
    "creatingText": "作成中...",
    "deleteFailed": "削除に失敗しました: {error}",
    "ideNotInstalled": "Cursor、VS Code、Antigravity のいずれもインストールされていません。",
    "ideNotInstalledTitle": "IDE が見つかりません",
    "ideLaunchFailed": "IDE の起動に失敗しました: {error}",
    "lfsWarning": "Git LFS のファイル取得に失敗したため、LFS ファイルをスキップしてワークツリーを作成しました。\nLFS ファイルが必要な場合は git lfs pull を実行してください。",
    "worktreeCreateFailed": "ワークツリーの作成に失敗しました: {error}"
  }
}
</i18n>
