<script setup lang="ts">
import { VueMonacoEditor } from "@guolao/vue-monaco-editor";

defineProps<{
  oldContent: string;
  newContent: string;
}>();

const options = {
  readOnly: true,
  minimap: { enabled: false },
  scrollBeyondLastLine: false,
  fontSize: 13,
  renderSideBySide: true,
};
</script>

<template>
  <VueMonacoEditor
    :original="oldContent"
    :value="newContent"
    :options="options"
    theme="vs-dark"
    diff-editor
    class="h-full w-full"
  />
</template>
