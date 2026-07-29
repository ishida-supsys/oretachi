<script setup lang="ts">
import { useI18n } from "vue-i18n";
import { usePanZoom } from "../../composables/usePanZoom";

const { t } = useI18n();

const {
  containerRef,
  contentRef,
  zoomPercent,
  dragging,
  transformStyle,
  zoomIn,
  zoomOut,
  resetZoom,
  fitToView,
} = usePanZoom();

// パン/ズームのイベントは createPanZoom 側が直接購読している。ただしドラッグ開始時に
// 既定動作 (テキスト選択) を止める都合でフォーカスが移らないため、ここで明示的に移す。
// これがないとクリック後もキーボードショートカットが効かない。
function onCanvasMouseDown() {
  containerRef.value?.focus();
}

function onKeydown(e: KeyboardEvent) {
  switch (e.key) {
    case "+":
    case "=":
      zoomIn();
      break;
    case "-":
      zoomOut();
      break;
    case "0":
      resetZoom();
      break;
    case "f":
    case "F":
      fitToView();
      break;
    default:
      return;
  }
  e.preventDefault();
}

// 親から描画完了後に初期フィットを呼べるようにする
defineExpose({ fitToView, resetZoom });
</script>

<template>
  <div
    ref="containerRef"
    class="pz-container"
    :class="{ dragging }"
    tabindex="0"
    @mousedown="onCanvasMouseDown"
    @keydown="onKeydown"
  >
    <div ref="contentRef" class="pz-content" :style="transformStyle">
      <slot />
    </div>

    <div class="pz-toolbar" data-ui>
      <button :title="t('panZoom.zoomOut')" @click="zoomOut">
        <span class="pi pi-search-minus" />
      </button>
      <span class="pz-level">{{ zoomPercent }}%</span>
      <button :title="t('panZoom.zoomIn')" @click="zoomIn">
        <span class="pi pi-search-plus" />
      </button>
      <span class="pz-sep" />
      <button :title="t('panZoom.fitToView')" @click="fitToView">
        <span class="pi pi-arrows-alt" />
      </button>
      <button :title="t('panZoom.resetZoom')" @click="resetZoom">
        <span class="pi pi-refresh" />
      </button>
    </div>
  </div>
</template>

<style scoped>
.pz-container {
  position: relative;
  height: 100%;
  width: 100%;
  overflow: hidden;
  cursor: grab;
  outline: none;
  user-select: none;
}

.pz-container.dragging {
  cursor: grabbing;
}

.pz-content {
  /* transform 基準を安定させるため、レイアウトはコンテンツ本来のサイズに任せる */
  position: absolute;
  top: 0;
  left: 0;
  width: max-content;
  will-change: transform;
}

/* ドラッグ中はコンテンツ側がマウスイベントを奪わないようにする */
.pz-container.dragging .pz-content {
  pointer-events: none;
}

.pz-toolbar {
  position: absolute;
  right: 12px;
  bottom: 12px;
  display: flex;
  align-items: center;
  gap: 2px;
  padding: 4px 6px;
  background: rgba(24, 24, 37, 0.85);
  border: 1px solid #313244;
  border-radius: 6px;
  opacity: 0.45;
  transition: opacity 0.15s;
}

.pz-container:hover .pz-toolbar,
.pz-container:focus-within .pz-toolbar {
  opacity: 1;
}

.pz-toolbar button {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  border: none;
  border-radius: 4px;
  background: transparent;
  color: #6c7086;
  cursor: pointer;
  transition: background 0.12s, color 0.12s;
}

.pz-toolbar button:hover {
  background: #313244;
  color: #cdd6f4;
}

.pz-toolbar button .pi {
  font-size: 12px;
}

.pz-level {
  min-width: 40px;
  text-align: center;
  color: #cdd6f4;
  font-size: 11px;
  font-variant-numeric: tabular-nums;
  user-select: none;
}

.pz-sep {
  width: 1px;
  height: 16px;
  margin: 0 4px;
  background: #313244;
}
</style>
