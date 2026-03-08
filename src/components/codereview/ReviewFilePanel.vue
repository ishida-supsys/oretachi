<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import Checkbox from "primevue/checkbox";
import MonacoDiffViewer from "./MonacoDiffViewer.vue";
import type { ReviewFileEntry } from "../../composables/useReviewSession";

const { t } = useI18n();

const props = defineProps<{ entry: ReviewFileEntry }>();
const emit = defineEmits<{
  (e: "toggle-reviewed"): void;
  (e: "toggle-collapsed"): void;
}>();

const statusColorClass: Record<string, string> = {
  M: "text-yellow-400",
  A: "text-green-400",
  D: "text-red-400",
  "??": "text-surface-400",
};

const diffHeight = computed(() => {
  const lines = Math.max(
    props.entry.oldContent.split("\n").length,
    props.entry.newContent.split("\n").length,
  );
  const px = Math.min(600, Math.max(200, lines * 19 + 40));
  return `${px}px`;
});

const checked = computed({
  get: () => props.entry.reviewed,
  set: () => emit("toggle-reviewed"),
});
</script>

<template>
  <div
    class="border border-surface-700 rounded overflow-hidden mb-2"
    :class="entry.reviewed ? 'opacity-60' : ''"
  >
    <!-- ヘッダー行 -->
    <div
      class="flex items-center gap-2 px-3 py-2 bg-surface-800 cursor-pointer select-none hover:bg-surface-750"
      @click="emit('toggle-collapsed')"
    >
      <!-- チェックボックス (クリックイベントを止める) -->
      <div @click.stop>
        <Checkbox v-model="checked" :binary="true" class="shrink-0" />
      </div>

      <!-- ファイルパス -->
      <span class="flex-1 text-sm font-mono text-surface-200 truncate">{{ entry.filePath }}</span>

      <!-- ステータスバッジ -->
      <span
        class="text-xs font-bold font-mono w-4 shrink-0"
        :class="statusColorClass[entry.status] ?? 'text-surface-300'"
      >{{ entry.status === "??" ? "?" : entry.status }}</span>

      <!-- レビュー済みラベル -->
      <span v-if="entry.reviewed" class="text-xs text-green-400 shrink-0">{{ t("reviewed") }}</span>

      <!-- 折りたたみアイコン -->
      <i
        class="pi text-surface-400 shrink-0 text-xs"
        :class="entry.collapsed ? 'pi-chevron-right' : 'pi-chevron-down'"
      />
    </div>

    <!-- Diff エリア -->
    <div v-if="!entry.collapsed" :style="{ height: diffHeight }">
      <MonacoDiffViewer :old-content="entry.oldContent" :new-content="entry.newContent" />
    </div>
  </div>
</template>

<i18n lang="json">
{
  "en": { "reviewed": "Reviewed" },
  "ja": { "reviewed": "レビュー済み" }
}
</i18n>
