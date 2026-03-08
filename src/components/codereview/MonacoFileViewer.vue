<script setup lang="ts">
import { computed } from "vue";
import { VueMonacoEditor } from "@guolao/vue-monaco-editor";
import { useEditorLineSelection, type ChatPayload } from "../../composables/useCodeReviewLineChat";
import EditorChatButton from "./EditorChatButton.vue";

const props = defineProps<{
  content: string;
  language?: string;
  filePath?: string;
}>();

const emit = defineEmits<{
  chat: [payload: ChatPayload];
}>();

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

const options = {
  readOnly: true,
  minimap: { enabled: true },
  scrollBeyondLastLine: false,
  fontSize: 13,
  lineNumbers: "on" as const,
  wordWrap: "off" as const,
  theme: "vs-dark",
};

const { buttonPos, handleMount, handleChatClick } = useEditorLineSelection(
  () => props.filePath,
  (payload) => emit("chat", payload),
);
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
    <EditorChatButton :button-pos="buttonPos" :file-path="filePath" @click="handleChatClick" />
  </div>
</template>
