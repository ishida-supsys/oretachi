<script setup lang="ts">
import { ref, watch, onMounted, nextTick } from "vue";
import mermaid from "mermaid";
import PanZoomCanvas from "./PanZoomCanvas.vue";
import { mermaidConfig, sanitizeMermaidSvg } from "../../utils/mermaidTheme";

const props = defineProps<{
  content: string;
}>();

const svgHtml = ref("");
const error = ref("");
const canvas = ref<InstanceType<typeof PanZoomCanvas> | null>(null);
let idCounter = 0;

mermaid.initialize(mermaidConfig);

async function render() {
  error.value = "";
  try {
    const id = `mermaid-${Date.now()}-${idCounter++}`;
    const { svg } = await mermaid.render(id, props.content);
    svgHtml.value = sanitizeMermaidSvg(svg);
    // SVG が DOM に載ってからサイズを測って全体が収まる倍率に合わせる
    await nextTick();
    canvas.value?.fitToView();
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
    <PanZoomCanvas v-else ref="canvas">
      <div class="mermaid-svg" v-html="svgHtml" />
    </PanZoomCanvas>
  </div>
</template>

<style scoped>
.mermaid-view {
  height: 100%;
  width: 100%;
  overflow: hidden;
  box-sizing: border-box;
}

/* 縮小によって文字が小さくなるのを避けるため、SVG は本来のサイズで描画して
   拡大縮小はキャンバス側の transform に任せる */
.mermaid-svg :deep(svg) {
  display: block;
  max-width: none;
}

.mermaid-error {
  color: #f38ba8;
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 13px;
  margin: 20px;
  padding: 12px;
  background: rgba(243, 139, 168, 0.1);
  border: 1px solid rgba(243, 139, 168, 0.3);
  border-radius: 6px;
}
</style>
