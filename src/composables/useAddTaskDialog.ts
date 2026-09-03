import { ref, computed, watch, onUnmounted, type Ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { useToast } from "primevue/usetoast";
import { useI18n } from "vue-i18n";
import type { ToastMessageOptions } from "primevue/toast";
import { useTasks } from "./useTasks";
import { useSettings } from "./useSettings";
import { useWorkgroups } from "./useWorkgroups";
import type { TaskCode, TaskProcessCode } from "../types/task";

/** add_worktree ステップでは生成された worktree ID を返す */
type StepExecutor = (code: TaskCode) => Promise<string | void>;

/** タスク完了からホームタブへ自動復帰するまでの待ち時間 */
const AUTO_RETURN_HOME_DELAY_MS = 5000;

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
    // executeAddWorktree は実行開始時の activeWorkgroupId で所属を決めるため、
    // 自動ホーム復帰の判定に使うグループもこの時点で確定させる
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
