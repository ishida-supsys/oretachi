<script setup lang="ts">
import { MdPreview } from "md-editor-v3";
import "md-editor-v3/lib/preview.css";

defineProps<{
  content: string;
}>();
</script>

<template>
  <div class="markdown-view">
    <MdPreview :modelValue="content" theme="dark" language="ja-JP" />
  </div>
</template>

<style scoped>
.markdown-view {
  overflow-y: auto;
  height: 100%;
  box-sizing: border-box;
  padding: 1rem 1.5rem;
}
</style>
