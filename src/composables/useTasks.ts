import { computed, ref } from "vue";
import type { TaskItem, TaskCode, TaskStatus, TaskStepStatus } from "../types/task";
import { persistedTasks, persistTask, deletePersisted } from "./useTaskPersistence";

// メモリ上の実行中タスク（generating / queued / executing）
const activeTasks = ref<TaskItem[]>([]);

// 全タスク: アクティブ(新しい順) + 永続化済み（重複排除）
const sortedTasks = computed(() => {
  const active = [...activeTasks.value].sort((a, b) => b.createdAt - a.createdAt);
  const activeIds = new Set(activeTasks.value.map((t) => t.id));
  const persisted = persistedTasks.value.filter((t) => !activeIds.has(t.id));
  return [...active, ...persisted];
});

function addTask(prompt: string): TaskItem {
  const task: TaskItem = {
    id: crypto.randomUUID(),
    prompt,
    createdAt: Date.now(),
    status: "generating",
    steps: [],
  };
  activeTasks.value.push(task);
  return task;
}

function setTaskSteps(taskId: string, codes: TaskCode[]) {
  const task = activeTasks.value.find((t) => t.id === taskId);
  if (!task) return;
  task.steps = codes.map((code) => ({ code, status: "pending" as TaskStepStatus }));
  task.status = "executing";
}

function updateStepStatus(
  taskId: string,
  stepIndex: number,
  status: TaskStepStatus,
  error?: string
) {
  const task = activeTasks.value.find((t) => t.id === taskId);
  if (!task || !task.steps[stepIndex]) return;
  task.steps[stepIndex].status = status;
  if (error !== undefined) task.steps[stepIndex].error = error;
}

async function updateTaskStatus(taskId: string, status: TaskStatus, error?: string) {
  const task = activeTasks.value.find((t) => t.id === taskId);
  if (!task) return;
  task.status = status;
  if (error !== undefined) task.error = error;

  // 完了またはエラー時に DB へ永続化し、activeTasks から移動
  if (status === "completed" || status === "error") {
    const index = activeTasks.value.findIndex((t) => t.id === taskId);
    if (index !== -1) activeTasks.value.splice(index, 1);
    await persistTask({ ...task });
  }
}

async function removeTask(taskId: string) {
  const activeIdx = activeTasks.value.findIndex((t) => t.id === taskId);
  if (activeIdx !== -1) {
    activeTasks.value.splice(activeIdx, 1);
  }
  await deletePersisted(taskId);
}

export function useTasks() {
  return {
    // 後方互換のため activeTasks を tasks として公開（useAddTaskDialog が参照）
    tasks: activeTasks,
    sortedTasks,
    addTask,
    setTaskSteps,
    updateStepStatus,
    updateTaskStatus,
    removeTask,
  };
}
