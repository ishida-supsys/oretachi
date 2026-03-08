<script setup lang="ts">
import { ref } from "vue";
import { useI18n } from "vue-i18n";
import GitChangesPane from "./GitChangesPane.vue";
import GitGraphPane from "./GitGraphPane.vue";

const { t } = useI18n();

const props = defineProps<{ repoPath: string }>();
const emit = defineEmits<{
  (e: "open-diff", payload: { filePath: string; staged: boolean }): void;
  (e: "start-review"): void;
}>();

type GitTab = "changes" | "graph";
const activeTab = ref<GitTab>("changes");
</script>

<template>
  <div class="h-full flex flex-col overflow-hidden">
    <!-- タブ切り替え -->
    <div class="flex shrink-0 border-b border-surface-700">
      <button
        class="flex-1 py-1.5 text-xs font-medium transition-colors"
        :class="activeTab === 'changes'
          ? 'text-primary-400 border-b-2 border-primary-400'
          : 'text-surface-400 hover:text-surface-200'"
        @click="activeTab = 'changes'"
      >
        {{ t("changes") }}
      </button>
      <button
        class="flex-1 py-1.5 text-xs font-medium transition-colors"
        :class="activeTab === 'graph'
          ? 'text-primary-400 border-b-2 border-primary-400'
          : 'text-surface-400 hover:text-surface-200'"
        @click="activeTab = 'graph'"
      >
        {{ t("graph") }}
      </button>
    </div>

    <div class="flex-1 overflow-hidden">
      <GitChangesPane
        v-if="activeTab === 'changes'"
        :repo-path="props.repoPath"
        @open-diff="emit('open-diff', $event)"
        @start-review="emit('start-review')"
      />
      <GitGraphPane
        v-else
        :repo-path="props.repoPath"
      />
    </div>
  </div>
</template>

<i18n lang="json">
{
  "en": { "changes": "Changes", "graph": "Graph" },
  "ja": { "changes": "変更", "graph": "グラフ" }
}
</i18n>
