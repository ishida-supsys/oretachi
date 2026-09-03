import { ref, computed, watch, onUnmounted, type Ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { useToast } from "primevue/usetoast";
import { useI18n } from "vue-i18n";
import type { ToastMessageOptions } from "primevue/toast";
import { useTasks } from "./useTasks";
import { useSettings } from "./useSettings";
import { useWorkgroups } from "./useWorkgroups";
import type { TaskCode, TaskProcessCode } from "../types/task";

type StepExecutor = (code: TaskCode) => Promise<void>;

/** タスク完了からホームタブへ自動復帰するまでの待ち時間 */
const AUTO_RETURN_HOME_DELAY_MS = 5000;

interface AutoReturnHomeOptions {
  /** メインウィンドウのフォーカス状態 */
  isWindowFocused: Ref<boolean>;
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

  const showAddTaskDialog = ref(false);
  const rerunTaskId = ref<string | null>(null);

  const rerunPrompt = computed(() => {
    if (!rerunTaskId.value) return "";
    return sortedTasks.value.find((t) => t.id === rerunTaskId.value)?.prompt ?? "";
  });

  let activeTaskToast: ToastMessageOptions | null = null;

  function showTaskToast(options: ToastMessageOptions): void {
    if (activeTaskToast) {
      toast.remove(activeTaskToast);
      activeTaskToast = null;
    }
    if (options.life === undefined) {
      activeTaskToast = options;
    }
    toast.add(options);
  }

  async function executeTaskSteps(taskId: string): Promise<void> {
    const { tasks } = useTasks();
    const task = tasks.value.find((t) => t.id === taskId);
    if (!task) return;

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
        await executeStep(step.code);
        updateStepStatus(taskId, i, "done");
      } catch (e) {
        const msg = e instanceof Error ? e.message : String(e);
        updateStepStatus(taskId, i, "error", msg);
        throw e;
      }
    }
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
   * - 対象ワークグループで autoReturnHomeAfterTask が有効なときのみ
   * - サブウィンドウ既定 (worktreeDefaults.openInSubWindow) ではメインのタブが動かないので対象外
   * - 完了時点でメインウィンドウがフォーカス済みなら、ユーザーが見ているので予約しない
   * - カウントダウン中にフォーカスされたらキャンセル
   */
  function scheduleAutoReturnHome(workgroupId: string | undefined): void {
    if (!autoReturnHome) return;
    const groupId = workgroupId ?? activeWorkgroupId.value;
    const group = settings.value.workgroups?.find((g) => g.id === groupId);
    if (!group?.autoReturnHomeAfterTask) return;
    if (settings.value.worktreeDefaults?.openInSubWindow) return;
    if (autoReturnHome.isWindowFocused.value) return;

    // 連続タスク実行で多重に張られないよう、既存の予約は張り直しでクリアする
    cancelAutoReturnHome();

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

  async function onAddTaskConfirm(prompt: string, remoteExec: boolean = false, workgroupId?: string): Promise<void> {
    const trimmed = prompt.trim();
    if (!trimmed) return;
    prompt = trimmed;
    showAddTaskDialog.value = false;
    rerunTaskId.value = null;
    // 追加先WGをアクティブにして、作成される worktree が現在のホーム表示に出るようにする
    if (workgroupId) {
      activeWorkgroupId.value = workgroupId;
    }
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

      await new Promise<void>((resolve, reject) => {
        executionQueue = executionQueue
          .catch(() => {})
          .then(async () => {
            updateTaskStatus(task.id, "executing");
            showTaskToast({
              severity: "info",
              summary: t("taskExecutingSummary"),
              detail: t("taskExecutingStartDetail", { count: stepCount }),
            });
            try {
              await executeTaskSteps(task.id);
              resolve();
            } catch (e) {
              reject(e);
            }
          });
      });

      updateTaskStatus(task.id, "completed");
      scheduleAutoReturnHome(workgroupId);

      showTaskToast({
        severity: "success",
        summary: t("taskCompletedSummary"),
        detail: t("taskCompletedDetail"),
        life: 3000,
      });
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      updateTaskStatus(task.id, "error", msg);
      // エラー内容をターミナルで確認する必要があるため、ホームへは戻さない
      cancelAutoReturnHome();

      showTaskToast({
        severity: "error",
        summary: t("taskFailedSummary"),
        detail: msg,
        life: 5000,
      });
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
