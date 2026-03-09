<script setup lang="ts">
import { computed, onMounted, onUnmounted } from "vue";
import { VueMonacoEditor } from "@guolao/vue-monaco-editor";
import { useEditorLineSelection, type ChatPayload } from "../../composables/useCodeReviewLineChat";
import { useCodeReviewSettings } from "../../composables/useCodeReviewSettings";
import { matchesHotkey } from "../../composables/useHotkeys";
import EditorChatButton from "./EditorChatButton.vue";

const props = defineProps<{
  content: string;
  language?: string;
  filePath?: string;
}>();

const emit = defineEmits<{
  chat: [payload: ChatPayload];
}>();

const { resolved: cr } = useCodeReviewSettings();

const languageMap: Record<string, string> = {
  ts: "typescript",
  tsx: "typescript",
  js: "javascript",
  jsx: "javascript",
  vue: "html",
  rs: "rust",
  py: "python",
  json: "json",
  md: "markdown",
  html: "html",
  css: "css",
  scss: "scss",
  toml: "ini",
  yaml: "yaml",
  yml: "yaml",
  sh: "shell",
  bash: "shell",
  go: "go",
  java: "java",
  cpp: "cpp",
  c: "c",
  h: "c",
  hpp: "cpp",
  kt: "kotlin",
  swift: "swift",
  rb: "ruby",
  php: "php",
  sql: "sql",
  xml: "xml",
};

const monacoLanguage = computed(() => {
  if (!props.language) return "plaintext";
  return languageMap[props.language.toLowerCase()] ?? "plaintext";
});

const chatHotkey = computed(() => cr.value.chatHotkey);

const options = computed(() => ({
  readOnly: true,
  minimap: { enabled: cr.value.monacoMinimap },
  scrollBeyondLastLine: false,
  fontSize: cr.value.monacoFontSize,
  lineNumbers: cr.value.monacoLineNumbers as "on" | "off",
  wordWrap: cr.value.monacoWordWrap as "on" | "off",
  theme: "vs-dark",
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
</script>

<template>
  <div class="relative h-full w-full">
    <VueMonacoEditor
      :value="content"
      :language="monacoLanguage"
      :options="options"
      theme="vs-dark"
      class="h-full w-full"
      @mount="handleMount"
    />
    <EditorChatButton :button-pos="buttonPos" :file-path="filePath" :hotkey="chatHotkey" @click="handleChatClick" />
  </div>
</template>
