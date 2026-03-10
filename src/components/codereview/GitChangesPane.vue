<script setup lang="ts">
import { ref, onMounted, onUnmounted } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useI18n } from "vue-i18n";
import { useReviewSession } from "../../composables/useReviewSession";

const { t } = useI18n();

const props = defineProps<{ repoPath: string }>();
const { isReviewMode } = useReviewSession();
const emit = defineEmits<{
  (e: "open-diff", payload: { filePath: string; staged: boolean }): void;
  (e: "start-review"): void;
}>();

interface StatusEntry {
  path: string;
  status: string;
  staged: boolean;
}

const stagedChanges = ref<StatusEntry[]>([]);
const unstagedChanges = ref<StatusEntry[]>([]);
const loading = ref(false);
const error = ref("");

const statusLabels: Record<string, string> = {
  M: "M",
  A: "A",
  D: "D",
  R: "R",
  C: "C",
  U: "U",
  "??": "?",
};

async function loadStatus() {
  if (!props.repoPath) return;
  loading.value = true;
  error.value = "";
  try {
    const entries = await invoke<StatusEntry[]>("git_get_status", { repoPath: props.repoPath });
    stagedChanges.value = entries.filter((e) => e.staged);
    unstagedChanges.value = entries.filter((e) => !e.staged);
  } catch (e) {
    error.value = String(e);
  } finally {
    loading.value = false;
  }
}

const statusColor: Record<string, string> = {
  M: "text-yellow-400",
  A: "text-green-400",
  D: "text-red-400",
  "??": "text-surface-400",
};

let unlistenFsChanged: (() => void) | null = null;
let debounceTimer: ReturnType<typeof setTimeout> | null = null;

onMounted(async () => {
  loadStatus();
  unlistenFsChanged = await listen("codereview-fs-changed", () => {
    if (debounceTimer !== null) clearTimeout(debounceTimer);
    debounceTimer = setTimeout(() => {
      debounceTimer = null;
      loadStatus();
    }, 500);
  });
});

onUnmounted(() => {
  if (debounceTimer !== null) {
    clearTimeout(debounceTimer);
    debounceTimer = null;
  }
  unlistenFsChanged?.();
});
</script>

<template>
  <div class="h-full overflow-auto p-2 text-sm">
    <div v-if="loading" class="text-surface-400 flex items-center gap-2 p-2">
      <i class="pi pi-spin pi-spinner" />{{ t("loading") }}
    </div>
    <div v-else-if="error" class="text-red-400 text-xs p-2">{{ error }}</div>
    <template v-else>
      <!-- ツールバー -->
      <div class="mb-2 flex items-center gap-2">
        <button
          class="flex items-center gap-1 text-xs text-surface-400 hover:text-surface-200 transition-colors"
          @click="loadStatus"
        >
          <i class="pi pi-refresh" />{{ t("refresh") }}
        </button>
        <button
          class="flex items-center gap-1 text-xs transition-colors"
          :class="stagedChanges.length === 0 && unstagedChanges.length === 0 || isReviewMode
            ? 'text-surface-600 cursor-not-allowed'
            : 'text-surface-400 hover:text-surface-200'"
          :disabled="stagedChanges.length === 0 && unstagedChanges.length === 0 || isReviewMode"
          @click="emit('start-review')"
        >
          <i class="pi pi-eye" />{{ t("review") }}
        </button>
      </div>

      <!-- Staged Changes -->
      <div class="mb-3">
        <div class="text-xs font-semibold text-surface-400 uppercase tracking-wider mb-1 px-1">
          {{ t("staged") }} ({{ stagedChanges.length }})
        </div>
        <div v-if="stagedChanges.length === 0" class="text-xs text-surface-500 px-2">
          {{ t("noChanges") }}
        </div>
        <div
          v-for="entry in stagedChanges"
          :key="`staged-${entry.path}`"
          class="flex items-center gap-2 px-2 py-0.5 rounded hover:bg-surface-700 cursor-pointer"
          @click="emit('open-diff', { filePath: entry.path, staged: true })"
        >
          <span
            class="text-xs font-mono font-bold w-4 shrink-0"
            :class="statusColor[entry.status] ?? 'text-surface-300'"
          >{{ statusLabels[entry.status] ?? entry.status }}</span>
          <span class="text-surface-200 truncate">{{ entry.path }}</span>
        </div>
      </div>

      <!-- Unstaged Changes -->
      <div>
        <div class="text-xs font-semibold text-surface-400 uppercase tracking-wider mb-1 px-1">
          {{ t("changes") }} ({{ unstagedChanges.length }})
        </div>
        <div v-if="unstagedChanges.length === 0" class="text-xs text-surface-500 px-2">
          {{ t("noChanges") }}
        </div>
        <div
          v-for="entry in unstagedChanges"
          :key="`unstaged-${entry.path}`"
          class="flex items-center gap-2 px-2 py-0.5 rounded hover:bg-surface-700 cursor-pointer"
          @click="emit('open-diff', { filePath: entry.path, staged: false })"
        >
          <span
            class="text-xs font-mono font-bold w-4 shrink-0"
            :class="statusColor[entry.status] ?? 'text-surface-300'"
          >{{ statusLabels[entry.status] ?? entry.status }}</span>
          <span class="text-surface-200 truncate">{{ entry.path }}</span>
        </div>
      </div>
    </template>
  </div>
</template>

<i18n lang="json">
{
  "en": {
    "loading": "Loading...",
    "refresh": "Refresh",
    "review": "Review",
    "staged": "Staged Changes",
    "changes": "Changes",
    "noChanges": "No changes"
  },
  "ja": {
    "loading": "読み込み中...",
    "refresh": "更新",
    "review": "レビュー",
    "staged": "ステージ済み",
    "changes": "変更",
    "noChanges": "変更なし"
  }
}
</i18n>
