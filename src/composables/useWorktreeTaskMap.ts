import { computed, ref } from "vue";
import { emit, listen } from "@tauri-apps/api/event";
import { useI18n } from "vue-i18n";
import { useTasks } from "./useTasks";
import { allPersistedTasks, loadAllTasks } from "./useTaskPersistence";
import type { TaskItem } from "../types/task";

const { tasks: activeTasks } = useTasks();

// 他ウィンドウ（主にメインウィンドウ）から受信した実行中タスク
const remoteActiveTasks = ref<TaskItem[]>([]);

// Tauri API の初期化（useWorktreeTaskMap() 呼び出し時に一度だけ実行）
let initialized = false;
function initTauriListeners() {
  if (initialized) return;
  initialized = true;

  // ページング・検索フィルタに影響されない全タスクを初回ロード
  loadAllTasks();

  // メインウィンドウでタスクが永続化/削除されたとき全ウィンドウで再ロード
  listen("task-data-changed", () => { loadAllTasks(); }).catch(() => {});

  // メインウィンドウの activeTasks をリアルタイム同期
  listen<TaskItem[]>("task-active-sync", (e) => {
    remoteActiveTasks.value = e.payload;
  }).catch(() => {});

  // 起動時に現在の activeTasks スナップショットをリクエスト（実行中タスクを即座に取得）
  emit("task-active-sync-request", {}).catch(() => {});
}

const worktreeTaskMap = computed(() => {
  // ローカルの activeTasks と他ウィンドウから受信した remoteActiveTasks を結合し、
  // さらに allPersistedTasks（全永続化済み）を加えて重複排除
  const seen = new Set<string>();
  const all: TaskItem[] = [];
  for (const t of [...activeTasks.value, ...remoteActiveTasks.value, ...allPersistedTasks.value]) {
    if (!seen.has(t.id)) {
      seen.add(t.id);
      all.push(t);
    }
  }

  const map = new Map<string, TaskItem[]>();
  for (const task of all) {
    const addedKeys = new Set<string>();
    for (const step of task.steps) {
      const key = `${step.code.repository}:${step.code.branch}`;
      if (addedKeys.has(key)) continue;
      addedKeys.add(key);
      if (!map.has(key)) map.set(key, []);
      map.get(key)!.push(task);
    }
  }
  return map;
});

export function useWorktreeTaskMap() {
  const { t } = useI18n();

  // Tauri 環境での初回呼び出し時にリスナーを登録
  initTauriListeners();

  function getTasksForWorktree(repositoryName: string, branchName: string): TaskItem[] {
    return worktreeTaskMap.value.get(`${repositoryName}:${branchName}`) ?? [];
  }

  function getTooltipText(repositoryName: string, branchName: string): string | undefined {
    // 最新のタスクを先頭に表示するため createdAt 降順でソート
    const tasks = getTasksForWorktree(repositoryName, branchName)
      .slice()
      .sort((a, b) => b.createdAt - a.createdAt);
    if (tasks.length === 0) return undefined;

    const maxShow = 3;
    const lines = tasks.slice(0, maxShow).map((task) => {
      const status = t(`taskStatus.${task.status}`, task.status);
      const prompt = task.prompt;
      const d = new Date(task.createdAt);
      const date = `${String(d.getMonth() + 1).padStart(2, "0")}/${String(d.getDate()).padStart(2, "0")} ${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
      return `[${status}] ${prompt} (${date})`;
    });
    if (tasks.length > maxShow) {
      lines.push(t("taskTooltipMore", { count: tasks.length - maxShow }));
    }
    return lines.join("<br>");
  }

  return { getTasksForWorktree, getTooltipText };
}
