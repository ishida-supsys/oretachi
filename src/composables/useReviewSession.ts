import { ref, computed } from "vue";
import { invoke } from "@tauri-apps/api/core";

export interface ReviewFileEntry {
  filePath: string;
  status: string;
  reviewed: boolean;
  collapsed: boolean;
  oldContent: string;
  newContent: string;
  isBinary: boolean;
}

interface StatusEntry {
  path: string;
  status: string;
  staged: boolean;
}

interface FileDiff {
  old_content: string;
  new_content: string;
  is_binary: boolean;
}

// モジュールレベルのシングルトン状態 (各 CodeReview ウィンドウは独立した JS ランタイム)
const isReviewMode = ref(false);
const reviewFiles = ref<ReviewFileEntry[]>([]);
const loading = ref(false);

const summary = computed(() => ({
  total: reviewFiles.value.length,
  reviewed: reviewFiles.value.filter((f) => f.reviewed).length,
}));

async function fetchDiff(repoPath: string, filePath: string, staged: boolean): Promise<FileDiff> {
  try {
    return await invoke<FileDiff>("git_get_file_diff", { repoPath, filePath, staged });
  } catch {
    return { old_content: "", new_content: "", is_binary: false };
  }
}

export function useReviewSession() {
  async function startReview(repoPath: string) {
    loading.value = true;
    try {
      const entries = await invoke<StatusEntry[]>("git_get_status", { repoPath });
      // ファイルパスの重複除去 (staged/unstaged 両方ある場合は unstaged 優先)
      const fileMap = new Map<string, StatusEntry>();
      for (const entry of entries) {
        if (!fileMap.has(entry.path) || !entry.staged) {
          fileMap.set(entry.path, entry);
        }
      }

      const files: ReviewFileEntry[] = await Promise.all(
        [...fileMap.values()].map(async (entry) => {
          const diff = await fetchDiff(repoPath, entry.path, entry.staged);
          return {
            filePath: entry.path,
            status: entry.status,
            reviewed: false,
            collapsed: false,
            oldContent: diff.old_content,
            newContent: diff.new_content,
            isBinary: diff.is_binary,
          };
        }),
      );

      reviewFiles.value = files;
      isReviewMode.value = true;
    } finally {
      loading.value = false;
    }
  }

  function endReview() {
    isReviewMode.value = false;
    reviewFiles.value = [];
  }

  function toggleReviewed(filePath: string) {
    const file = reviewFiles.value.find((f) => f.filePath === filePath);
    if (!file) return;
    file.reviewed = !file.reviewed;
    if (file.reviewed) {
      file.collapsed = true;
    }
  }

  function toggleCollapsed(filePath: string) {
    const file = reviewFiles.value.find((f) => f.filePath === filePath);
    if (file) {
      file.collapsed = !file.collapsed;
    }
  }

  async function refreshReviewFiles(repoPath: string) {
    if (!isReviewMode.value) return;
    try {
      const entries = await invoke<StatusEntry[]>("git_get_status", { repoPath });

      const fileMap = new Map<string, StatusEntry>();
      for (const entry of entries) {
        if (!fileMap.has(entry.path) || !entry.staged) {
          fileMap.set(entry.path, entry);
        }
      }
      const newPaths = new Set(fileMap.keys());

      // 削除されたファイルを除去
      reviewFiles.value = reviewFiles.value.filter((f) => newPaths.has(f.filePath));

      // 新規追加 + 既存更新
      const updatedFiles = await Promise.all(
        [...fileMap.values()].map(async (entry) => {
          const existing = reviewFiles.value.find((f) => f.filePath === entry.path);
          const diff = await fetchDiff(repoPath, entry.path, entry.staged);

          if (existing) {
            if (existing.oldContent !== diff.old_content || existing.newContent !== diff.new_content) {
              // diff が変わっていたらレビュー済みをリセット
              return { ...existing, oldContent: diff.old_content, newContent: diff.new_content, isBinary: diff.is_binary, reviewed: false, collapsed: false };
            }
            return existing;
          } else {
            return {
              filePath: entry.path,
              status: entry.status,
              reviewed: false,
              collapsed: false,
              oldContent: diff.old_content,
              newContent: diff.new_content,
              isBinary: diff.is_binary,
            };
          }
        }),
      );

      reviewFiles.value = updatedFiles;
    } catch {
      /* 無視 */
    }
  }

  async function commitAll(repoPath: string, message: string) {
    await invoke("git_stage_all", { repoPath });
    await invoke<string>("git_commit", { repoPath, message });
    endReview();
  }

  return {
    isReviewMode,
    reviewFiles,
    loading,
    summary,
    startReview,
    endReview,
    toggleReviewed,
    toggleCollapsed,
    refreshReviewFiles,
    commitAll,
  };
}
