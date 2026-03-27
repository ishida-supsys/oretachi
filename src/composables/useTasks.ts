import { ref, computed } from "vue";
import { invoke } from "@tauri-apps/api/core";
import type { TaskItem, TaskCode, TaskStatus, TaskStepStatus } from "../types/task";

// Rust の TaskRow に対応する型（snake_case で受け渡し）
interface TaskRow {
  id: string;
  prompt: string;
  created_at: number;
  status: string;
  steps: string; // JSON string: TaskStep[]
  error: string | null;
}

interface TaskListResult {
  items: TaskRow[];
  has_more: boolean;
}

function taskRowToItem(row: TaskRow): TaskItem {
  return {
    id: row.id,
    prompt: row.prompt,
    createdAt: row.created_at,
    status: row.status as TaskStatus,
    steps: JSON.parse(row.steps),
    error: row.error ?? undefined,
  };
}

function taskItemToRow(task: TaskItem): TaskRow {
  return {
    id: task.id,
    prompt: task.prompt,
    created_at: task.createdAt,
    status: task.status,
    steps: JSON.stringify(task.steps),
    error: task.error ?? null,
  };
}

const PAGE_SIZE = 30;

// メモリ上の実行中タスク（generating / queued / executing）
const activeTasks = ref<TaskItem[]>([]);

// DB から取得した完了済み/エラーのタスク
const persistedTasks = ref<TaskItem[]>([]);

// 検索とページネーション状態
const searchQuery = ref("");
const currentOffset = ref(0);
const hasMore = ref(true);
const isLoading = ref(false);

// 全タスク: アクティブ(新しい順) + 永続化済み（重複排除）
const sortedTasks = computed(() => {
  const active = [...activeTasks.value].sort((a, b) => b.createdAt - a.createdAt);
  const activeIds = new Set(activeTasks.value.map((t) => t.id));
  const persisted = persistedTasks.value.filter((t) => !activeIds.has(t.id));
  return [...active, ...persisted];
});

// DB からタスクを取得（reset=true で先頭から再ロード）
async function loadTasks(reset = false): Promise<void> {
  if (isLoading.value) return;
  if (!reset && !hasMore.value) return;

  isLoading.value = true;
  try {
    if (reset) {
      currentOffset.value = 0;
      persistedTasks.value = [];
      hasMore.value = true;
    }
    const result = await invoke<TaskListResult>("list_tasks", {
      search: searchQuery.value,
      offset: currentOffset.value,
      limit: PAGE_SIZE,
    });
    const newItems = result.items.map(taskRowToItem);
    persistedTasks.value.push(...newItems);
    currentOffset.value += newItems.length;
    hasMore.value = result.has_more;
  } catch (e) {
    console.error("Failed to load tasks:", e);
  } finally {
    isLoading.value = false;
  }
}

// 次ページをロード（無限スクロール用）
async function loadMore(): Promise<void> {
  await loadTasks(false);
}

// 検索クエリ変更 → リセットしてロード
async function search(query: string): Promise<void> {
  searchQuery.value = query;
  await loadTasks(true);
}

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

  // 完了またはエラー時に DB へ永続化し、activeTasks から persistedTasks へ移動
  if (status === "completed" || status === "error") {
    try {
      await invoke("save_task", { task: taskItemToRow(task) });
    } catch (e) {
      console.error("Failed to save task to DB:", e);
    }

    const index = activeTasks.value.findIndex((t) => t.id === taskId);
    if (index !== -1) activeTasks.value.splice(index, 1);

    // DB保存の成否に関わらず persistedTasks の先頭に追加して表示を維持
    persistedTasks.value.unshift({ ...task });
    currentOffset.value += 1;
  }
}

async function removeTask(taskId: string) {
  // activeTasks から除去
  const activeIdx = activeTasks.value.findIndex((t) => t.id === taskId);
  if (activeIdx !== -1) {
    activeTasks.value.splice(activeIdx, 1);
  }
  // persistedTasks から除去（offset は巻き戻さない: 次の loadMore で重複を防ぐため）
  const persistedIdx = persistedTasks.value.findIndex((t) => t.id === taskId);
  if (persistedIdx !== -1) {
    persistedTasks.value.splice(persistedIdx, 1);
  }
  // DB から削除
  try {
    await invoke("delete_task", { id: taskId });
  } catch (e) {
    console.error("Failed to delete task from DB:", e);
  }
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
    // 新規: ページネーション・検索
    searchQuery,
    hasMore,
    isLoading,
    loadTasks,
    loadMore,
    search,
  };
}
