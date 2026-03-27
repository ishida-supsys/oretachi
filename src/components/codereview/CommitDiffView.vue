<script setup lang="ts">
import { ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { invoke } from "@tauri-apps/api/core";
import ReviewFilePanel from "./ReviewFilePanel.vue";
import type { ReviewFileEntry } from "../../composables/useReviewSession";
import type { ChatPayload } from "../../composables/useCodeReviewLineChat";

const { t } = useI18n();

const props = defineProps<{ repoPath: string; commitHash: string }>();
const emit = defineEmits<{ (e: "chat", payload: ChatPayload): void }>();

interface CommitFileEntryRaw {
  path: string;
  status: string;
}

const files = ref<ReviewFileEntry[]>([]);
const loading = ref(false);
const error = ref("");

function toggleCollapsed(filePath: string) {
  const f = files.value.find((e) => e.filePath === filePath);
  if (f) f.collapsed = !f.collapsed;
}

async function loadDiff(hash: string) {
  loading.value = true;
  error.value = "";
  files.value = [];
  try {
    const entries = await invoke<CommitFileEntryRaw[]>("git_get_commit_files", {
      repoPath: props.repoPath,
      hash,
    });

    const loaded: ReviewFileEntry[] = await Promise.all(
      entries.map(async (entry) => {
        try {
          const diff = await invoke<{ old_content: string; new_content: string; is_binary: boolean }>(
            "git_get_commit_file_diff",
            { repoPath: props.repoPath, hash, filePath: entry.path },
          );
          return {
            filePath: entry.path,
            status: entry.status,
            reviewed: false,
            collapsed: false,
            oldContent: diff.old_content,
            newContent: diff.new_content,
            isBinary: diff.is_binary,
          };
        } catch {
          return {
            filePath: entry.path,
            status: entry.status,
            reviewed: false,
            collapsed: false,
            oldContent: "",
            newContent: "",
            isBinary: false,
          };
        }
      }),
    );
    files.value = loaded;
  } catch (e) {
    error.value = String(e);
  } finally {
    loading.value = false;
  }
}

watch(() => props.commitHash, loadDiff, { immediate: true });
</script>

<template>
  <div class="flex flex-col h-full overflow-hidden">
    <!-- ヘッダー -->
    <div class="flex items-center gap-3 px-4 py-2 bg-surface-800 border-b border-surface-700 shrink-0">
      <span class="font-mono text-xs text-primary-400 shrink-0">{{ commitHash.slice(0, 7) }}</span>
      <span class="text-sm text-surface-300">
        {{ t("filesChanged", { count: files.length }) }}
      </span>
    </div>

    <!-- ファイル一覧 -->
    <div class="flex-1 overflow-auto p-3">
      <div v-if="loading" class="flex items-center justify-center h-full text-surface-400 text-sm">
        <i class="pi pi-spin pi-spinner mr-2" />{{ t("loading") }}
      </div>
      <div v-else-if="error" class="p-3 text-red-400 text-xs">{{ error }}</div>
      <div v-else-if="files.length === 0" class="flex items-center justify-center h-full text-surface-500 text-sm">
        {{ t("noFiles") }}
      </div>
      <ReviewFilePanel
        v-for="entry in files"
        v-else
        :key="entry.filePath"
        :entry="entry"
        :readonly="true"
        @toggle-reviewed="() => {}"
        @toggle-collapsed="toggleCollapsed(entry.filePath)"
        @chat="(payload) => emit('chat', payload)"
      />
    </div>
  </div>
</template>

<i18n lang="json">
{
  "en": { "filesChanged": "{count} files changed", "loading": "Loading diff...", "noFiles": "No changed files" },
  "ja": { "filesChanged": "{count} ファイル変更", "loading": "差分を読み込み中...", "noFiles": "変更ファイルなし" }
}
</i18n>
