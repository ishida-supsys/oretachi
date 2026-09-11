<script setup lang="ts">
import { computed } from "vue";
import { VueMonacoEditor } from "@guolao/vue-monaco-editor";
// loader.config({ monaco }) の副作用 import。これが無いと @monaco-editor/loader が
// 既定の CDN から monaco を取りに行き、オフラインではエディタが空のままになる
// (アーティファクトビューワーの webview は CodeReviewApp を読まないため、ここで張る)
import "../../monaco-workers";

const props = defineProps<{
  content: string;
  language?: string;
}>();

const monacoLanguage = computed(() => props.language ?? "plaintext");
</script>

<template>
  <div class="code-view">
    <VueMonacoEditor
      :value="content"
      :language="monacoLanguage"
      theme="vs-dark"
      :options="{
        readOnly: true,
        minimap: { enabled: false },
        scrollBeyondLastLine: false,
        wordWrap: 'on',
        fontSize: 13,
      }"
    />
  </div>
</template>

<style scoped>
.code-view {
  height: 100%;
  width: 100%;
}
</style>
