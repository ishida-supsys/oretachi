<script setup lang="ts">
import { computed, onMounted, ref, watch, nextTick } from "vue";
import DOMPurify from "dompurify";
import PanZoomCanvas from "./PanZoomCanvas.vue";

const props = defineProps<{
  content: string;
}>();

const canvas = ref<InstanceType<typeof PanZoomCanvas> | null>(null);

const sanitized = computed(() =>
  DOMPurify.sanitize(props.content, { USE_PROFILES: { svg: true, svgFilters: true } })
);

async function fitAfterRender() {
  await nextTick();
  canvas.value?.fitToView();
}

onMounted(fitAfterRender);
watch(sanitized, fitAfterRender);
</script>

<template>
  <div class="svg-view">
    <PanZoomCanvas ref="canvas">
      <div class="svg-container" v-html="sanitized" />
    </PanZoomCanvas>
  </div>
</template>

<style scoped>
.svg-view {
  height: 100%;
  width: 100%;
  overflow: hidden;
}

/* 拡大縮小はキャンバス側の transform に任せ、SVG は本来のサイズで描画する */
.svg-container :deep(svg) {
  display: block;
  max-width: none;
  max-height: none;
}
</style>
