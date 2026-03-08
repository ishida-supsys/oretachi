<script setup lang="ts">
import { computed } from "vue";
import { VueMonacoEditor } from "@guolao/vue-monaco-editor";

const props = defineProps<{
  content: string;
  language?: string;
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
</script>

<template>
  <VueMonacoEditor
    :value="content"
    :language="monacoLanguage"
    :options="options"
    theme="vs-dark"
    class="h-full w-full"
  />
</template>
