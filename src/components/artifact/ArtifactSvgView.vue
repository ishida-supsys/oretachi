<script setup lang="ts">
import { computed } from "vue";
import DOMPurify from "dompurify";

const props = defineProps<{
  content: string;
}>();

const sanitized = computed(() =>
  DOMPurify.sanitize(props.content, { USE_PROFILES: { svg: true, svgFilters: true } })
);
</script>

<template>
  <div class="svg-view">
    <div class="svg-container" v-html="sanitized" />
  </div>
</template>

<style scoped>
.svg-view {
  height: 100%;
  width: 100%;
  overflow: auto;
  display: flex;
  align-items: center;
  justify-content: center;
}

.svg-container :deep(svg) {
  max-width: 100%;
  max-height: 100%;
}
</style>
