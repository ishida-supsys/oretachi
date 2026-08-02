<script setup lang="ts">
import { computed, onMounted, ref, watch, nextTick } from "vue";
import DOMPurify from "dompurify";
import PanZoomCanvas from "./PanZoomCanvas.vue";

const props = defineProps<{
  content: string;
}>();

const root = ref<HTMLElement | null>(null);
const canvas = ref<InstanceType<typeof PanZoomCanvas> | null>(null);

const sanitized = computed(() =>
  DOMPurify.sanitize(props.content, { USE_PROFILES: { svg: true, svgFilters: true } })
);

/** width/height が未指定または百分率か (= レイアウトサイズが確定しない) */
function isIndefinite(value: string | null): boolean {
  return !value || value.trim().endsWith("%");
}

/**
 * ルート svg のサイズを px に確定させる。
 *
 * パンズームキャンバスは `width: max-content` の絶対配置なので、`width="100%"` のような
 * 百分率指定だと SVG の既定置換サイズ (300x150) に潰れて中身がクリップされてしまう。
 * viewBox があればその寸法を、なければビューポートいっぱいの px を与える。
 */
function normalizeSvgSize() {
  const el = root.value;
  const svg = el?.querySelector("svg");
  if (!el || !svg) return;
  const width = svg.getAttribute("width");
  const height = svg.getAttribute("height");
  if (!isIndefinite(width) && !isIndefinite(height)) return;

  const box = svg.viewBox.baseVal;
  if (box && box.width > 0 && box.height > 0) {
    svg.setAttribute("width", String(box.width));
    svg.setAttribute("height", String(box.height));
    return;
  }
  // viewBox がない場合は本来のサイズが分からないので、従来どおりビューポートを埋める
  if (el.clientWidth > 0 && el.clientHeight > 0) {
    svg.setAttribute("width", String(el.clientWidth));
    svg.setAttribute("height", String(el.clientHeight));
  }
}

async function fitAfterRender() {
  await nextTick();
  normalizeSvgSize();
  canvas.value?.fitToView();
}

onMounted(fitAfterRender);
watch(sanitized, fitAfterRender);
</script>

<template>
  <div ref="root" class="svg-view">
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
