<script setup lang="ts">
import { ref, onMounted, onUnmounted } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useI18n } from "vue-i18n";

const { t } = useI18n();

const props = defineProps<{ repoPath: string }>();

interface CommitEntry {
  hash: string;
  short_hash: string;
  author: string;
  date: string;
  message: string;
  parents: string[];
  refs: string[];
}

const commits = ref<CommitEntry[]>([]);
const loading = ref(false);
const loadingMore = ref(false);
const error = ref("");
const hasMore = ref(true);
const PAGE_SIZE = 50;

function formatDate(dateStr: string): string {
  try {
    return new Date(dateStr).toLocaleDateString("ja-JP", {
      year: "numeric",
      month: "2-digit",
      day: "2-digit",
    });
  } catch {
    return dateStr.substring(0, 10);
  }
}

async function loadCommits(skip = 0) {
  if (!props.repoPath) return;
  try {
    const items = await invoke<CommitEntry[]>("git_get_log", {
      repoPath: props.repoPath,
      skip,
      limit: PAGE_SIZE,
    });
    if (skip === 0) {
      commits.value = items;
    } else {
      commits.value.push(...items);
    }
    hasMore.value = items.length === PAGE_SIZE;
  } catch (e) {
    error.value = String(e);
  }
}

async function init() {
  loading.value = true;
  await loadCommits(0);
  loading.value = false;
}

async function loadMore() {
  if (loadingMore.value || !hasMore.value) return;
  loadingMore.value = true;
  await loadCommits(commits.value.length);
  loadingMore.value = false;
}

function onScroll(e: Event) {
  const el = e.target as HTMLElement;
  if (el.scrollHeight - el.scrollTop - el.clientHeight < 100) {
    loadMore();
  }
}

let unlistenFsChanged: (() => void) | null = null;
let debounceTimer: ReturnType<typeof setTimeout> | null = null;

onMounted(async () => {
  await init();
  unlistenFsChanged = await listen("codereview-fs-changed", () => {
    if (debounceTimer !== null) clearTimeout(debounceTimer);
    debounceTimer = setTimeout(() => {
      debounceTimer = null;
      loadCommits(0);
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
  <div class="h-full overflow-auto text-sm" @scroll="onScroll">
    <div v-if="loading" class="flex items-center justify-center p-4 text-surface-400">
      <i class="pi pi-spin pi-spinner mr-2" />{{ t("loading") }}
    </div>
    <div v-else-if="error" class="p-3 text-red-400 text-xs">{{ error }}</div>
    <template v-else>
      <div
        v-for="commit in commits"
        :key="commit.hash"
        class="px-3 py-2 border-b border-surface-700 hover:bg-surface-700 cursor-default"
      >
        <div class="flex items-start gap-2">
          <span class="font-mono text-xs text-primary-400 shrink-0 mt-0.5 w-14">
            {{ commit.short_hash }}
          </span>
          <div class="flex-1 min-w-0">
            <div class="text-surface-100 truncate">{{ commit.message }}</div>
            <div class="flex items-center gap-2 mt-0.5">
              <span class="text-surface-400 text-xs">{{ commit.author }}</span>
              <span class="text-surface-500 text-xs">{{ formatDate(commit.date) }}</span>
            </div>
            <!-- ref バッジ -->
            <div v-if="commit.refs.length > 0" class="flex flex-wrap gap-1 mt-1">
              <span
                v-for="ref in commit.refs"
                :key="ref"
                class="px-1.5 py-0.5 rounded text-xs font-mono"
                :class="ref.startsWith('HEAD') ? 'bg-primary-700 text-primary-100' : 'bg-surface-600 text-surface-200'"
              >
                {{ ref }}
              </span>
            </div>
          </div>
        </div>
      </div>
      <div v-if="loadingMore" class="flex items-center justify-center py-3 text-surface-400 text-sm">
        <i class="pi pi-spin pi-spinner mr-2" />
      </div>
      <div v-if="!hasMore && commits.length > 0" class="text-center py-3 text-surface-500 text-xs">
        {{ t("end") }}
      </div>
    </template>
  </div>
</template>

<i18n lang="json">
{
  "en": { "loading": "Loading commits...", "end": "All commits loaded" },
  "ja": { "loading": "コミットを読み込み中...", "end": "すべてのコミットを読み込みました" }
}
</i18n>
