<script setup lang="ts">
import { computed } from "vue";
import { VueMonacoEditor } from "@guolao/vue-monaco-editor";

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
