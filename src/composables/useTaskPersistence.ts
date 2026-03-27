import { ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import type { TaskItem, TaskStatus } from "../types/task";

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

// DB から取得した完了済み/エラーのタスク
export const persistedTasks = ref<TaskItem[]>([]);

// 検索とページネーション状態
export const searchQuery = ref("");
const currentOffset = ref(0);
const hasMore = ref(true);
const isLoading = ref(false);

// DB に実際に存在するタスクIDを追跡（offset 管理の整合性のため）
const dbBackedIds = new Set<string>();

// ツールチップ用: ページング・検索に影響されない全タスク
export const allPersistedTasks = ref<TaskItem[]>([]);

export async function loadAllTasks(): Promise<void> {
  try {
    const result = await invoke<TaskListResult>("list_tasks", {
      search: "",
      offset: 0,
      limit: 10000,
    });
    allPersistedTasks.value = result.items.map(taskRowToItem);
  } catch (e) {
    console.error("Failed to load all tasks:", e);
  }
}

// reset 要求がロード中に来た場合はペンディングして完了後に再実行
let pendingReset = false;

async function loadTasks(reset = false): Promise<void> {
  if (isLoading.value) {
    if (reset) pendingReset = true;
    return;
  }
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
    for (const item of newItems) dbBackedIds.add(item.id);
    persistedTasks.value.push(...newItems);
    currentOffset.value += newItems.length;
    hasMore.value = result.has_more;
  } catch (e) {
    console.error("Failed to load tasks:", e);
  } finally {
    isLoading.value = false;
  }

  if (pendingReset) {
    pendingReset = false;
    await loadTasks(true);
  }
}

async function loadMore(): Promise<void> {
  await loadTasks(false);
}

async function search(query: string): Promise<void> {
  searchQuery.value = query;
  await loadTasks(true);
}

/** タスクを DB に保存し、persistedTasks の先頭に追加する */
export async function persistTask(task: TaskItem): Promise<void> {
  let savedToDb = false;
  try {
    await invoke("save_task", { task: taskItemToRow(task) });
    savedToDb = true;
    dbBackedIds.add(task.id);
  } catch (e) {
    console.error("Failed to save task to DB:", e);
  }

  const matchesSearch =
    !searchQuery.value ||
    task.prompt.toLowerCase().includes(searchQuery.value.toLowerCase());
  if (matchesSearch) {
    persistedTasks.value.unshift({ ...task });
    if (savedToDb) currentOffset.value += 1;
  }

  // allPersistedTasks にも追加（重複しない場合のみ）
  if (!allPersistedTasks.value.some((t) => t.id === task.id)) {
    allPersistedTasks.value.unshift({ ...task });
  }
}

/** persistedTasks からタスクを削除し、DB からも削除する */
export async function deletePersisted(taskId: string): Promise<void> {
  const idx = persistedTasks.value.findIndex((t) => t.id === taskId);
  let removed: TaskItem | undefined;
  if (idx !== -1) {
    removed = persistedTasks.value[idx];
    persistedTasks.value.splice(idx, 1);
  }

  const wasDbBacked = dbBackedIds.has(taskId);
  try {
    await invoke("delete_task", { id: taskId });
    dbBackedIds.delete(taskId);
    if (removed !== undefined && wasDbBacked) {
      currentOffset.value = Math.max(0, currentOffset.value - 1);
    }
    // allPersistedTasks からも削除
    const allIdx = allPersistedTasks.value.findIndex((t) => t.id === taskId);
    if (allIdx !== -1) allPersistedTasks.value.splice(allIdx, 1);
  } catch (e) {
    console.error("Failed to delete task from DB:", e);
    if (removed !== undefined && idx !== -1) {
      persistedTasks.value.splice(idx, 0, removed);
    }
  }
}

export function useTaskPersistence() {
  return {
    persistedTasks,
    allPersistedTasks,
    searchQuery,
    hasMore,
    isLoading,
    loadTasks,
    loadAllTasks,
    loadMore,
    search,
    persistTask,
    deletePersisted,
  };
}
