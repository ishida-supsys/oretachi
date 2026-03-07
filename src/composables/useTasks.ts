import { ref, computed } from "vue";
import type { TaskItem, TaskCode, TaskStatus, TaskStepStatus } from "../types/task";

const tasks = ref<TaskItem[]>([]);

const sortedTasks = computed(() =>
  [...tasks.value].sort((a, b) => b.createdAt - a.createdAt)
);

function addTask(prompt: string): TaskItem {
  const task: TaskItem = {
    id: crypto.randomUUID(),
    prompt,
    createdAt: Date.now(),
    status: "generating",
    steps: [],
  };
  tasks.value.push(task);
  return task;
}

function setTaskSteps(taskId: string, codes: TaskCode[]) {
  const task = tasks.value.find((t) => t.id === taskId);
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
  const task = tasks.value.find((t) => t.id === taskId);
  if (!task || !task.steps[stepIndex]) return;
  task.steps[stepIndex].status = status;
  if (error !== undefined) task.steps[stepIndex].error = error;
}

function updateTaskStatus(taskId: string, status: TaskStatus, error?: string) {
  const task = tasks.value.find((t) => t.id === taskId);
  if (!task) return;
  task.status = status;
  if (error !== undefined) task.error = error;
}

function removeTask(taskId: string) {
  const index = tasks.value.findIndex((t) => t.id === taskId);
  if (index !== -1) tasks.value.splice(index, 1);
}

export function useTasks() {
  return {
    tasks,
    sortedTasks,
    addTask,
    setTaskSteps,
    updateStepStatus,
    updateTaskStatus,
    removeTask,
  };
}
