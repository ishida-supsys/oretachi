<script setup lang="ts">
import { VueMonacoDiffEditor } from "@guolao/vue-monaco-editor";

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
  <VueMonacoDiffEditor
    :original="oldContent"
    :modified="newContent"
    :options="options"
    theme="vs-dark"
    class="h-full w-full"
  />
</template>
