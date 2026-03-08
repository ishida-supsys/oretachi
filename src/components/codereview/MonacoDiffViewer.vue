<script setup lang="ts">
import type * as Monaco from "monaco-editor";
import { VueMonacoDiffEditor } from "@guolao/vue-monaco-editor";
import { useEditorLineSelection, type ChatPayload } from "../../composables/useCodeReviewLineChat";
import EditorChatButton from "./EditorChatButton.vue";

const props = defineProps<{
  oldContent: string;
  newContent: string;
  filePath?: string;
}>();

const emit = defineEmits<{
  chat: [payload: ChatPayload];
}>();

const options = {
  readOnly: true,
  minimap: { enabled: false },
  scrollBeyondLastLine: false,
  fontSize: 13,
  renderSideBySide: true,
};

const { buttonPos, handleMount, handleChatClick } = useEditorLineSelection(
  () => props.filePath,
  (payload) => emit("chat", payload),
);

function onMount(editor: Monaco.editor.IStandaloneDiffEditor) {
  handleMount(editor.getModifiedEditor());
}
</script>

<template>
  <div class="relative h-full w-full">
    <VueMonacoDiffEditor
      :original="oldContent"
      :modified="newContent"
      :options="options"
      theme="vs-dark"
      class="h-full w-full"
      @mount="onMount"
    />
    <EditorChatButton :button-pos="buttonPos" :file-path="filePath" @click="handleChatClick" />
  </div>
</template>
