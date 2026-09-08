import { ref, computed, watch, onUnmounted, type Ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { useToast } from "primevue/usetoast";
import { useI18n } from "vue-i18n";
import type { ToastMessageOptions } from "primevue/toast";
import { useTasks } from "./useTasks";
import { useSettings } from "./useSettings";
import { useWorkgroups } from "./useWorkgroups";
import { useWorktrees } from "./useWorktrees";
import { useNotifications, playSoundForKind, sendOsNotification } from "./useNotifications";
import { resolveKindSetting } from "../utils/notificationKinds";
import { buildTrayNotificationMap } from "../utils/trayNotification";
import { HOME_WORKTREE_ID, isHomeWorktree } from "../utils/homeWorktree";
import type { TaskCode, TaskProcessCode } from "../types/task";

/** add_worktree ステップでは生成された worktree ID を返す */
type StepExecutor = (code: TaskCode) => Promise<string | void>;

/** タスク完了からホームタブへ自動復帰するまでの待ち時間 */
const AUTO_RETURN_HOME_DELAY_MS = 5000;

/** OS 通知の本文に載せる失敗理由の上限。AI CLI の stderr 全文が来ることがある（#223） */
const NOTIFICATION_DETAIL_MAX_LEN = 200;

/** OS 通知向けに失敗理由を刈る。全文はトーストとタスク一覧（`error`）に残る。 */
function truncateForNotification(detail: string): string {
  const oneLine = detail.replace(/\s+/g, " ").trim();
  return oneLine.length > NOTIFICATION_DETAIL_MAX_LEN
    ? `${oneLine.slice(0, NOTIFICATION_DETAIL_MAX_LEN)}…`
    : oneLine;
}

interface AutoReturnHomeOptions {
  /** メインウィンドウのフォーカス状態 */
  isWindowFocused: Ref<boolean>;
  /** サブウィンドウへ移されたワークツリーか（メインのタブが動かないので対象外にする） */
  isDetached: (worktreeId: string) => boolean;
  /** ホームタブへ戻す */
  goHome: () => void;
}

let executionQueue: Promise<void> = Promise.resolve();

export function useAddTaskDialog(executeStep: StepExecutor, autoReturnHome?: AutoReturnHomeOptions) {
  const toast = useToast();
  const { t } = useI18n();
  const { settings, scheduleSave } = useSettings();
  const { activeWorkgroupId } = useWorkgroups();
  const { sortedTasks, addTask, setTaskSteps, updateStepStatus, updateTaskStatus } = useTasks();
  const { addNotification } = useNotifications();
  const { worktrees } = useWorktrees();

  const showAddTaskDialog = ref(false);
  const rerunTaskId = ref<string | null>(null);

  const rerunPrompt = computed(() => {
    if (!rerunTaskId.value) return "";
    return sortedTasks.value.find((t) => t.id === rerunTaskId.value)?.prompt ?? "";
  });

  let activeTaskToast: ToastMessageOptions | null = null;
  /** 失敗トーストの枠。進捗トーストとは別に持つ理由は showTaskToast のコメント参照。 */
  let activeErrorToast: ToastMessageOptions | null = null;

  function showTaskToast(options: ToastMessageOptions): void {
    // 進捗トーストは常に1枚へ畳む。失敗トーストだけは**別枠**で保持し、後続タスクの
    // 進捗トーストでは消さない（#223）。同じ枠に入れると、次のタスクを投げた時点で
    // 失敗の痕跡がトーストから消え、席を外している人には何も残らない。
    // 失敗トースト同士は最新1枚に畳む（連続失敗で積み上がると閉じるのが手間）。
    const isError = options.severity === "error";
    const slot = isError ? activeErrorToast : activeTaskToast;
    if (slot) toast.remove(slot);
    // life 付き（自動で消える）トーストは追跡しない。remove する前に消えている
    const tracked = options.life === undefined ? options : null;
    if (isError) activeErrorToast = tracked;
    else activeTaskToast = tracked;
    toast.add(options);
  }

  /** 全ステップを実行し、add_worktree で生成された worktree ID を返す（生成なしなら null） */
  async function executeTaskSteps(taskId: string): Promise<string | null> {
    const { tasks } = useTasks();
    const task = tasks.value.find((t) => t.id === taskId);
    if (!task) return null;

    let createdWorktreeId: string | null = null;

    for (let i = 0; i < task.steps.length; i++) {
      const step = task.steps[i];
      updateStepStatus(taskId, i, "running");

      const stepLabel = step.code.type === "add_worktree"
        ? t("taskStepAddWorktree")
        : t("taskStepAgent");
      showTaskToast({
        severity: "info",
        summary: t("taskExecutingSummary"),
        detail: t("taskStepDetail", { current: i + 1, total: task.steps.length, label: stepLabel }),
      });

      try {
        const result = await executeStep(step.code);
        if (step.code.type === "add_worktree" && typeof result === "string") {
          createdWorktreeId = result;
        }
        updateStepStatus(taskId, i, "done");
      } catch (e) {
        const msg = e instanceof Error ? e.message : String(e);
        updateStepStatus(taskId, i, "error", msg);
        throw e;
      }
    }

    return createdWorktreeId;
  }

  let autoReturnHomeTimer: ReturnType<typeof setTimeout> | null = null;
  let autoReturnHomeUnwatch: (() => void) | null = null;

  /** 予約済みの自動ホーム復帰を破棄する */
  function cancelAutoReturnHome(): void {
    if (autoReturnHomeTimer !== null) {
      clearTimeout(autoReturnHomeTimer);
      autoReturnHomeTimer = null;
    }
    if (autoReturnHomeUnwatch) {
      autoReturnHomeUnwatch();
      autoReturnHomeUnwatch = null;
    }
  }

  /**
   * タスク完了後、一定時間でホームタブへ戻す予約を入れる。
   * - ワークツリーを生成したタスクのみ（既存ワークツリーへの agent_worktree のみのタスクはタブが動かない）
   * - 対象ワークグループで autoReturnHomeAfterTask が有効なときのみ
   * - サブウィンドウへ移された（メインのタブが動かない）ワークツリーは対象外
   * - 完了時点でメインウィンドウがフォーカス済みなら、ユーザーが見ているので予約しない
   * - カウントダウン中にフォーカスされたらキャンセル
   */
  function scheduleAutoReturnHome(groupId: string | undefined, createdWorktreeId: string | null): void {
    if (!autoReturnHome) return;
    if (!createdWorktreeId) return;
    const group = settings.value.workgroups?.find((g) => g.id === groupId);
    if (!group?.autoReturnHomeAfterTask) return;
    if (autoReturnHome.isDetached(createdWorktreeId)) return;
    if (autoReturnHome.isWindowFocused.value) return;

    const { isWindowFocused, goHome } = autoReturnHome;
    autoReturnHomeUnwatch = watch(isWindowFocused, (focused) => {
      if (focused) cancelAutoReturnHome();
    });
    autoReturnHomeTimer = setTimeout(() => {
      autoReturnHomeTimer = null;
      cancelAutoReturnHome();
      goHome();
    }, AUTO_RETURN_HOME_DELAY_MS);
  }

  onUnmounted(cancelAutoReturnHome);

  /**
   * @param switchActiveWorkgroup 追加先WGをホームのアクティブWGにするか。
   *   UI のタスク追加ダイアログからは true（ユーザーが選んだ WG をそのまま見せる）。
   *   MCP の oretachi_add_task 由来は false（UI の表示状態と無関係に発生する追加なので、
   *   ユーザーが見ているワークグループを勝手に切り替えない。#181）。
   */
  async function onAddTaskConfirm(
    prompt: string,
    remoteExec: boolean = false,
    workgroupId?: string,
    switchActiveWorkgroup: boolean = true,
  ): Promise<void> {
    const trimmed = prompt.trim();
    if (!trimmed) return;
    prompt = trimmed;
    showAddTaskDialog.value = false;
    rerunTaskId.value = null;
    // 追加先WGをアクティブにして、作成される worktree が現在のホーム表示に出るようにする
    // （切り替えは activeWorkgroupId の永続化を伴うため、MCP 由来では行わない）
    if (workgroupId && switchActiveWorkgroup) {
      activeWorkgroupId.value = workgroupId;
    }
    // ワークグループ指定がないとき executeAddWorktree は実行開始時の activeWorkgroupId で
    // 所属を決めるため、自動ホーム復帰の判定に使うグループもこの時点で確定させる
    const autoReturnGroupId = workgroupId ?? activeWorkgroupId.value;
    if (settings.value.aiAgent) {
      settings.value.aiAgent.remoteExec = remoteExec;
    } else {
      settings.value.aiAgent = { remoteExec };
    }
    scheduleSave();
    const task = addTask(prompt);

    showTaskToast({
      severity: "info",
      summary: t("taskAddSummary"),
      detail: t("taskAddDetail"),
    });

    try {
      const result = await invoke<string>("task_generate", { prompt });
      const taskProcessCode = JSON.parse(result) as TaskProcessCode;
      if (remoteExec) {
        for (const code of taskProcessCode.code) {
          if (code.type === "agent_worktree") {
            code.remoteExec = true;
          }
        }
      }
      if (workgroupId) {
        for (const code of taskProcessCode.code) {
          if (code.type === "add_worktree") {
            code.workgroupId = workgroupId;
          }
        }
      }
      setTaskSteps(task.id, taskProcessCode.code);

      const stepCount = taskProcessCode.code.length;
      updateTaskStatus(task.id, "queued");

      const createdWorktreeId = await new Promise<string | null>((resolve, reject) => {
        executionQueue = executionQueue
          .catch(() => {})
          .then(async () => {
            // 後続タスクが走り出したら、先行タスクの予約は無効化する
            // （このタスクが新しいタブを開くので、そこへホームを被せてはいけない）
            cancelAutoReturnHome();
            updateTaskStatus(task.id, "executing");
            showTaskToast({
              severity: "info",
              summary: t("taskExecutingSummary"),
              detail: t("taskExecutingStartDetail", { count: stepCount }),
            });
            try {
              resolve(await executeTaskSteps(task.id));
            } catch (e) {
              reject(e);
            }
          });
      });

      updateTaskStatus(task.id, "completed");
      scheduleAutoReturnHome(autoReturnGroupId, createdWorktreeId);

      showTaskToast({
        severity: "success",
        summary: t("taskCompletedSummary"),
        detail: t("taskCompletedDetail"),
        life: 3000,
      });
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      updateTaskStatus(task.id, "error", msg);
      // エラー時は scheduleAutoReturnHome を通らないので予約は張られない。
      // ここで cancelAutoReturnHome() すると別タスクの正当な予約まで消すのでしない。

      // 自動で消さない。5 秒で消えると席を外している間の失敗が痕跡ごと消える（#223）
      showTaskToast({
        severity: "error",
        summary: t("taskFailedSummary"),
        detail: msg,
      });
      await notifyTaskFailure(task.id, msg);
    }
  }

  /**
   * 失敗したタスクの通知先ワークツリーを決める（#223）。
   *
   * タスクが必ずしもワークツリーを持たないわけではない。`add_worktree` が成功した後に
   * `agent_worktree` で落ちる場合や、既存ワークツリーへの `agent_worktree` だけの
   * タスクでは対象が実在するので、そちらへバッジを積んだほうが人が辿りやすい。
   * ステップの repository / branch とワークツリーの突き合わせ規則は
   * `useTaskExecution` の `agent_worktree` 実行時と同じ。
   *
   * 突き合わせ先は**ランタイムの `worktrees`**（`settings.value.worktrees` ではない）。
   * 両者は乖離しうる: `add_worktree` 失敗時の `rollbackWorktree` はランタイムからしか
   * 削除しないので、settings 側で引くと「カードに出ていないワークツリー」へバッジを
   * 積んでしまう（`purgeStaleNotifications` もランタイム基準なので後で消える）。
   *
   * 生成そのものが失敗した場合は steps が空なのでホームへ落ちる。
   */
  function resolveFailureWorktreeId(taskId: string): string {
    const { tasks } = useTasks();
    const steps = tasks.value.find((tk) => tk.id === taskId)?.steps ?? [];
    // 落ちたステップを優先して見る（複数ワークツリーに跨るタスクでも当事者へ届く）
    const ordered = [...steps].sort((a, b) => Number(b.status === "error") - Number(a.status === "error"));
    for (const step of ordered) {
      const wt = worktrees.value.find(
        (w) => w.repositoryName === step.code.repository && w.branchName === step.code.branch,
      );
      if (wt) return wt.id;
    }
    return HOME_WORKTREE_ID;
  }

  /**
   * タスクの生成・実行が失敗したことを人に届ける（#223）。
   *
   * トーストだけでは足りない。MCP の `oretachi_add_task` 由来のタスクは投げっぱなしで
   * 実行されるので、メインウィンドウを見ていない間に失敗すると誰も気付かず、
   * `oretachi_list_tasks` を明示的に叩くまで分からなかった。
   *
   * `notify-worktree` へ相乗りさせないこと。あれは全 MCP ピアへブロードキャストされ、
   * 自動承認の AI 判定まで走らせてしまう。
   *
   * **この関数は投げない。** 呼び出し元は fire-and-forget な
   * `mcp-add-task` リスナー（App.vue）でもあり、ここで reject すると
   * unhandled rejection になる。通知が出せなくても task status とトーストは
   * 既に確定しているので、黙って諦めるのが正しい。
   */
  async function notifyTaskFailure(taskId: string, detail: string): Promise<void> {
    try {
      // 種別ごとの ON/OFF と、通知先ワークツリーの trayNotification を尊重する
      if (!resolveKindSetting(settings.value.notificationSound, "general").enabled) return;
      const worktreeId = resolveFailureWorktreeId(taskId);
      if (!(buildTrayNotificationMap(settings.value).get(worktreeId) ?? true)) return;
      addNotification(worktreeId, "general");
      playSoundForKind("general");
      // クリック時のフォーカス先は**名前**で解決されるので名前を渡し、本文だけ理由に差し替える。
      // 理由は task_executor が AI CLI の stderr 全文を載せてくることがあり、
      // OS 通知（Windows ではアクションセンターに残る）へ丸ごと流すと読めないので刈る。
      const target =
        worktrees.value.find((w) => w.id === worktreeId) ??
        worktrees.value.find(isHomeWorktree);
      await sendOsNotification(
        target?.name ?? "home",
        t("notification.titleTaskFailed"),
        "general",
        truncateForNotification(detail),
      );
    } catch {
      // 通知経路の失敗は握りつぶす（上のコメント参照）
    }
  }

  function onAddTaskCancel(): void {
    showAddTaskDialog.value = false;
    rerunTaskId.value = null;
  }

  return {
    showAddTaskDialog,
    rerunTaskId,
    rerunPrompt,
    showTaskToast,
    onAddTaskConfirm,
    onAddTaskCancel,
  };
}
