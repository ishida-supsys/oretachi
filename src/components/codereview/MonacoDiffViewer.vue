<script setup lang="ts">
import type * as Monaco from "monaco-editor";
import { computed, onMounted, onUnmounted } from "vue";
import { VueMonacoDiffEditor } from "@guolao/vue-monaco-editor";
import { useEditorLineSelection, type ChatPayload } from "../../composables/useCodeReviewLineChat";
import { useCodeReviewSettings } from "../../composables/useCodeReviewSettings";
import { matchesHotkey } from "../../composables/useHotkeys";
import EditorChatButton from "./EditorChatButton.vue";

const props = defineProps<{
  oldContent: string;
  newContent: string;
  filePath?: string;
  autoHeight?: boolean;
}>();

const emit = defineEmits<{
  chat: [payload: ChatPayload];
  contentHeightChange: [height: number];
}>();

const { resolved: cr } = useCodeReviewSettings();

const chatHotkey = computed(() => cr.value.chatHotkey);

const options = computed(() => ({
  readOnly: true,
  minimap: { enabled: cr.value.monacoMinimap },
  scrollBeyondLastLine: false,
  fontSize: cr.value.monacoFontSize,
  renderSideBySide: true,
  hideUnchangedRegions: { enabled: true },
}));

const { buttonPos, handleMount, handleChatClick, selectionInfo } = useEditorLineSelection(
  () => props.filePath,
  (payload) => emit("chat", payload),
);

function onKeydown(e: KeyboardEvent) {
  if (!selectionInfo.value) return;
  if (matchesHotkey(e, chatHotkey.value)) {
    e.preventDefault();
    handleChatClick();
  }
}

onMounted(() => window.addEventListener("keydown", onKeydown, true));
onUnmounted(() => window.removeEventListener("keydown", onKeydown, true));

function onMount(editor: Monaco.editor.IStandaloneDiffEditor) {
  handleMount(editor.getModifiedEditor());

  if (props.autoHeight) {
    editor.getModifiedEditor().onDidContentSizeChange((e) => {
      emit("contentHeightChange", e.contentHeight);
    });
  }
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
    <EditorChatButton :button-pos="buttonPos" :file-path="filePath" :hotkey="chatHotkey" @click="handleChatClick" />
  </div>
</template>
