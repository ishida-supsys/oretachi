import { ref, computed } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { useToast } from "primevue/usetoast";
import { useI18n } from "vue-i18n";
import type { ToastMessageOptions } from "primevue/toast";
import { useTasks } from "./useTasks";
import { useSettings } from "./useSettings";
import type { TaskCode, TaskProcessCode } from "../types/task";

type StepExecutor = (code: TaskCode) => Promise<void>;

let executionQueue: Promise<void> = Promise.resolve();

export function useAddTaskDialog(executeStep: StepExecutor) {
  const toast = useToast();
  const { t } = useI18n();
  const { settings, scheduleSave } = useSettings();
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

  async function onAddTaskConfirm(prompt: string, remoteExec: boolean = false): Promise<void> {
    showAddTaskDialog.value = false;
    rerunTaskId.value = null;
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

      showTaskToast({
        severity: "success",
        summary: t("taskCompletedSummary"),
        detail: t("taskCompletedDetail"),
        life: 3000,
      });
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      updateTaskStatus(task.id, "error", msg);

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
