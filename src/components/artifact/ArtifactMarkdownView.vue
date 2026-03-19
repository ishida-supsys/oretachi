<script setup lang="ts">
import { computed } from "vue";
import { marked } from "marked";
import DOMPurify from "dompurify";

const props = defineProps<{
  content: string;
}>();

const html = computed(() => {
  const raw = marked.parse(props.content) as string;
  return DOMPurify.sanitize(raw);
});
</script>

<template>
  <div class="markdown-view" v-html="html" />
</template>

<style scoped>
.markdown-view {
  padding: 20px 24px;
  color: #cdd6f4;
  line-height: 1.7;
  overflow-y: auto;
  height: 100%;
  box-sizing: border-box;
}

.markdown-view :deep(h1),
.markdown-view :deep(h2),
.markdown-view :deep(h3),
.markdown-view :deep(h4) {
  color: #cba6f7;
  margin: 1.2em 0 0.4em;
}

.markdown-view :deep(h1) { font-size: 1.6em; }
.markdown-view :deep(h2) { font-size: 1.3em; }
.markdown-view :deep(h3) { font-size: 1.1em; }

.markdown-view :deep(code) {
  background: #313244;
  border-radius: 3px;
  padding: 1px 5px;
  font-size: 0.9em;
  font-family: monospace;
}

.markdown-view :deep(pre) {
  background: #1e1e2e;
  border: 1px solid #313244;
  border-radius: 6px;
  padding: 12px 16px;
  overflow-x: auto;
}

.markdown-view :deep(pre code) {
  background: none;
  padding: 0;
}

.markdown-view :deep(a) {
  color: #89b4fa;
}

.markdown-view :deep(blockquote) {
  border-left: 3px solid #45475a;
  margin: 0;
  padding-left: 16px;
  color: #a6adc8;
}

.markdown-view :deep(table) {
  border-collapse: collapse;
  width: 100%;
}

.markdown-view :deep(th),
.markdown-view :deep(td) {
  border: 1px solid #313244;
  padding: 6px 12px;
  text-align: left;
}

.markdown-view :deep(th) {
  background: #313244;
}

.markdown-view :deep(ul),
.markdown-view :deep(ol) {
  padding-left: 1.5em;
}

.markdown-view :deep(hr) {
  border: none;
  border-top: 1px solid #313244;
  margin: 1em 0;
}
</style>
