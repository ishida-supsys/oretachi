<script setup lang="ts">
import { ref, watch, onMounted } from "vue";
import mermaid from "mermaid";

const props = defineProps<{
  content: string;
}>();

const svgHtml = ref("");
const error = ref("");
let idCounter = 0;

mermaid.initialize({
  startOnLoad: false,
  theme: "dark",
  themeVariables: {
    background: "#1e1e2e",
    primaryColor: "#cba6f7",
    primaryTextColor: "#cdd6f4",
    lineColor: "#6c7086",
  },
});

async function render() {
  error.value = "";
  try {
    const id = `mermaid-${Date.now()}-${idCounter++}`;
    const { svg } = await mermaid.render(id, props.content);
    svgHtml.value = svg;
  } catch (e) {
    error.value = String(e);
    svgHtml.value = "";
  }
}

onMounted(render);
watch(() => props.content, render);
</script>

<template>
  <div class="mermaid-view">
    <div v-if="error" class="mermaid-error">
      <span class="pi pi-exclamation-triangle" />
      {{ error }}
    </div>
    <div v-else class="mermaid-svg" v-html="svgHtml" />
  </div>
</template>

<style scoped>
.mermaid-view {
  height: 100%;
  width: 100%;
  overflow: auto;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 20px;
  box-sizing: border-box;
}

.mermaid-svg :deep(svg) {
  max-width: 100%;
}

.mermaid-error {
  color: #f38ba8;
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 13px;
  padding: 12px;
  background: rgba(243, 139, 168, 0.1);
  border: 1px solid rgba(243, 139, 168, 0.3);
  border-radius: 6px;
}
</style>
