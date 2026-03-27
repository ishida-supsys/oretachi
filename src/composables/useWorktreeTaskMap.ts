import { computed, ref } from "vue";
import { listen } from "@tauri-apps/api/event";
import { useTasks } from "./useTasks";
import { allPersistedTasks, loadAllTasks } from "./useTaskPersistence";
import type { TaskItem } from "../types/task";

const statusLabel: Record<string, string> = {
  generating: "生成中",
  queued: "待機中",
  executing: "実行中",
  completed: "完了",
  error: "エラー",
};

const { tasks: activeTasks } = useTasks();

// 他ウィンドウ（主にメインウィンドウ）から受信した実行中タスク
const remoteActiveTasks = ref<TaskItem[]>([]);

// ページング・検索フィルタに影響されない全タスクを初回ロード
loadAllTasks();

// メインウィンドウでタスクが永続化/削除されたとき全ウィンドウで再ロード
listen("task-data-changed", () => { loadAllTasks(); }).catch(() => {});

// メインウィンドウの activeTasks をリアルタイム同期
listen<TaskItem[]>("task-active-sync", (e) => {
  remoteActiveTasks.value = e.payload;
}).catch(() => {});

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
    for (const step of task.steps) {
      const key = `${step.code.repository}:${step.code.branch}`;
      if (!map.has(key)) map.set(key, []);
      map.get(key)!.push(task);
    }
  }
  return map;
});

export function useWorktreeTaskMap() {
  function getTasksForWorktree(repositoryName: string, branchName: string): TaskItem[] {
    return worktreeTaskMap.value.get(`${repositoryName}:${branchName}`) ?? [];
  }

  function getTooltipText(repositoryName: string, branchName: string): string | undefined {
    const tasks = getTasksForWorktree(repositoryName, branchName);
    if (tasks.length === 0) return undefined;

    const maxShow = 3;
    const lines = tasks.slice(0, maxShow).map((task) => {
      const status = statusLabel[task.status] ?? task.status;
      const prompt =
        task.prompt.length > 30 ? task.prompt.slice(0, 30) + "..." : task.prompt;
      const d = new Date(task.createdAt);
      const date = `${String(d.getMonth() + 1).padStart(2, "0")}/${String(d.getDate()).padStart(2, "0")} ${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
      return `[${status}] ${prompt} (${date})`;
    });
    if (tasks.length > maxShow) {
      lines.push(`...他${tasks.length - maxShow}件`);
    }
    return lines.join("\n");
  }

  return { getTasksForWorktree, getTooltipText };
}
