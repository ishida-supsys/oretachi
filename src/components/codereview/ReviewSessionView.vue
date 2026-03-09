<script setup lang="ts">
import { ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { useToast } from "primevue/usetoast";
import { invoke } from "@tauri-apps/api/core";
import ReviewFilePanel from "./ReviewFilePanel.vue";
import { useReviewSession } from "../../composables/useReviewSession";
import type { ChatPayload } from "../../composables/useCodeReviewLineChat";

const { t } = useI18n();
const toast = useToast();

const props = defineProps<{ repoPath: string }>();
const emit = defineEmits<{
  (e: "close"): void;
  (e: "chat", payload: ChatPayload): void;
}>();

const { reviewFiles, summary, toggleReviewed, toggleCollapsed, commitAll } = useReviewSession();

// コミットポップオーバー
const showCommitPanel = ref(false);
const commitMessage = ref("");
const committing = ref(false);
const generatingMessage = ref(false);

// レビュー開始時、変更ファイルがあればコミットメッセージを自動生成
const stopAutoGenerate = watch(
  () => reviewFiles.value.length,
  (len) => {
    if (len > 0 && !commitMessage.value && !generatingMessage.value) {
      generateCommitMessage();
      stopAutoGenerate();
    }
  },
  { immediate: true },
);

async function generateCommitMessage() {
  if (generatingMessage.value) {
    await invoke("cancel_commit_message_generation", { repoPath: props.repoPath });
    generatingMessage.value = false;
    return;
  }
  generatingMessage.value = true;
  try {
    const message = await invoke<string>("generate_commit_message", { repoPath: props.repoPath });
    commitMessage.value = message;
  } catch (e) {
    if (String(e) !== "already in progress") {
      toast.add({ severity: "error", summary: t("generateFailed"), detail: String(e), life: 5000 });
    }
  } finally {
    generatingMessage.value = false;
  }
}

async function handleCommit() {
  if (!commitMessage.value.trim()) return;
  committing.value = true;
  try {
    await commitAll(props.repoPath, commitMessage.value.trim());
    commitMessage.value = "";
    showCommitPanel.value = false;
    toast.add({ severity: "success", summary: t("commitSuccess"), life: 3000 });
    emit("close");
  } catch (e) {
    toast.add({ severity: "error", summary: t("commitFailed"), detail: String(e), life: 5000 });
  } finally {
    committing.value = false;
  }
}
</script>

<template>
  <div class="flex flex-col h-full overflow-hidden">
    <!-- ヘッダー -->
    <div class="flex items-center gap-3 px-4 py-2 bg-surface-800 border-b border-surface-700 shrink-0">
      <!-- 左: 進捗 -->
      <div class="flex-1 flex items-center gap-3 text-sm">
        <span class="text-surface-300 font-medium">
          {{ t("filesChanged", { count: summary.total }) }}
        </span>
        <span class="text-surface-400 text-xs">
          {{ t("reviewedProgress", { done: summary.reviewed, total: summary.total }) }}
        </span>
      </div>

      <!-- 右: ボタン群 -->
      <div class="flex items-center gap-2 relative">
        <!-- コミットボタン -->
        <button
          class="px-3 py-1 text-xs font-medium rounded bg-primary-600 hover:bg-primary-500 text-white transition-colors"
          @click="showCommitPanel = !showCommitPanel"
        >
          <i class="pi pi-check mr-1" />{{ t("commit") }}
        </button>

        <!-- 閉じるボタン -->
        <button
          class="px-3 py-1 text-xs font-medium rounded bg-surface-700 hover:bg-surface-600 text-surface-200 transition-colors"
          @click="emit('close')"
        >
          <i class="pi pi-times mr-1" />{{ t("close") }}
        </button>

        <!-- コミットポップオーバー -->
        <div
          v-if="showCommitPanel"
          class="absolute top-full right-0 mt-1 w-80 bg-surface-800 border border-surface-600 rounded shadow-xl z-50 p-3"
        >
          <div class="flex items-center justify-between mb-2">
            <span class="text-xs font-semibold text-surface-300">{{ t("commitMessage") }}</span>
            <button
              class="flex items-center gap-1 px-2 py-0.5 text-xs rounded bg-surface-700 hover:bg-surface-600 text-surface-300 hover:text-surface-100 transition-colors"
              :title="generatingMessage ? t('cancel') : t('generateCommitMessage')"
              @click="generateCommitMessage"
            >
              <i v-if="generatingMessage" class="pi pi-spin pi-spinner text-xs" />
              <i v-else class="pi pi-sparkles text-xs" />
              <span>{{ generatingMessage ? t("generating") : t("generateCommitMessage") }}</span>
            </button>
          </div>
          <textarea
            v-model="commitMessage"
            class="w-full bg-surface-900 border border-surface-600 rounded px-2 py-1.5 text-sm text-surface-100 resize-none focus:outline-none focus:border-primary-500"
            rows="3"
            :placeholder="t('commitPlaceholder')"
            @keydown.ctrl.enter="handleCommit"
          />
          <div class="flex justify-end gap-2 mt-2">
            <button
              class="px-3 py-1 text-xs rounded bg-surface-700 hover:bg-surface-600 text-surface-200 transition-colors"
              @click="showCommitPanel = false"
            >
              {{ t("cancel") }}
            </button>
            <button
              class="px-3 py-1 text-xs rounded bg-primary-600 hover:bg-primary-500 text-white transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
              :disabled="!commitMessage.trim() || committing"
              @click="handleCommit"
            >
              <i v-if="committing" class="pi pi-spin pi-spinner mr-1" />
              {{ t("commitRun") }}
            </button>
          </div>
        </div>
      </div>
    </div>

    <!-- ファイル一覧 -->
    <div class="flex-1 overflow-auto p-3">
      <div v-if="reviewFiles.length === 0" class="flex items-center justify-center h-full text-surface-500 text-sm">
        {{ t("noChanges") }}
      </div>
      <ReviewFilePanel
        v-for="entry in reviewFiles"
        :key="entry.filePath"
        :entry="entry"
        @toggle-reviewed="toggleReviewed(entry.filePath)"
        @toggle-collapsed="toggleCollapsed(entry.filePath)"
        @chat="(payload) => emit('chat', payload)"
      />
    </div>
  </div>
</template>

<i18n lang="json">
{
  "en": {
    "filesChanged": "{count} files changed",
    "reviewedProgress": "{done}/{total} reviewed",
    "commit": "Commit",
    "close": "Close Review",
    "commitMessage": "Commit message",
    "commitPlaceholder": "Enter commit message (Ctrl+Enter to commit)",
    "cancel": "Cancel",
    "commitRun": "Commit",
    "noChanges": "No changed files",
    "commitSuccess": "Committed successfully",
    "commitFailed": "Commit failed",
    "generateCommitMessage": "Generate",
    "generating": "Generating...",
    "generateFailed": "Failed to generate commit message"
  },
  "ja": {
    "filesChanged": "{count} ファイル変更",
    "reviewedProgress": "{done}/{total} レビュー済み",
    "commit": "コミット",
    "close": "レビューを閉じる",
    "commitMessage": "コミットメッセージ",
    "commitPlaceholder": "コミットメッセージを入力 (Ctrl+Enter でコミット)",
    "cancel": "キャンセル",
    "commitRun": "コミット",
    "noChanges": "変更ファイルなし",
    "commitSuccess": "コミットしました",
    "commitFailed": "コミットに失敗しました",
    "generateCommitMessage": "生成",
    "generating": "生成中...",
    "generateFailed": "コミットメッセージの生成に失敗しました"
  }
}
</i18n>
