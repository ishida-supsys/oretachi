<script setup lang="ts">
import type { CodeReviewTab } from "../../composables/useCodeReviewTabs";

defineProps<{
  tabs: CodeReviewTab[];
  activeTabId: string;
}>();

const emit = defineEmits<{
  (e: "switch", id: string): void;
  (e: "close", id: string): void;
}>();
</script>

<template>
  <div
    v-if="tabs.length > 0"
    class="flex items-center gap-0 bg-surface-900 border-b border-surface-700 overflow-x-auto shrink-0"
  >
    <div
      v-for="tab in tabs"
      :key="tab.id"
      class="flex items-center gap-1.5 px-3 py-1.5 text-sm cursor-pointer border-r border-surface-700 whitespace-nowrap select-none shrink-0"
      :class="tab.id === activeTabId
        ? 'bg-surface-800 text-surface-100 border-t-2 border-t-primary-400'
        : 'text-surface-400 hover:bg-surface-800 hover:text-surface-200'"
      @click="emit('switch', tab.id)"
    >
      <i :class="tab.type === 'file' ? 'pi pi-file text-xs' : 'pi pi-share-alt text-xs'" />
      <span>{{ tab.label }}</span>
      <button
        class="ml-1 rounded hover:bg-surface-600 p-0.5 leading-none"
        @click.stop="emit('close', tab.id)"
      >
        <i class="pi pi-times text-xs" />
      </button>
    </div>
  </div>
</template>
