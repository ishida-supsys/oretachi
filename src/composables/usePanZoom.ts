import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { createPanZoom, type PanZoomController, type PanZoomOptions } from "../utils/panZoom";

export type { PanZoomOptions } from "../utils/panZoom";

/**
 * SVG などのコンテンツに対するパン/ズームを Vue コンポーネントに繋ぐ。
 *
 * 操作ロジックそのものは utils/panZoom.ts の createPanZoom が持ち、
 * ここでは状態を ref に写してテンプレートから使えるようにするだけ。
 * containerRef をビューポート、contentRef を transform 対象に割り当てて使う。
 */
export function usePanZoom(options: Omit<PanZoomOptions, "onChange"> = {}) {
  const containerRef = ref<HTMLElement | null>(null);
  const contentRef = ref<HTMLElement | null>(null);

  const pan = ref({ x: 0, y: 0 });
  const zoom = ref(1);
  const dragging = ref(false);

  let controller: PanZoomController | null = null;

  const transformStyle = computed(() => ({
    transform: `translate(${pan.value.x}px, ${pan.value.y}px) scale(${zoom.value})`,
    transformOrigin: "0 0",
  }));

  const zoomPercent = computed(() => Math.round(zoom.value * 100));

  onMounted(() => {
    if (!containerRef.value) return;
    controller = createPanZoom(containerRef.value, () => contentRef.value, {
      ...options,
      onChange: (s) => {
        pan.value = { x: s.x, y: s.y };
        zoom.value = s.zoom;
        dragging.value = s.dragging;
      },
    });
  });

  onBeforeUnmount(() => {
    controller?.destroy();
    controller = null;
  });

  return {
    containerRef,
    contentRef,
    pan,
    zoom,
    zoomPercent,
    dragging,
    transformStyle,
    zoomIn: () => controller?.zoomIn(),
    zoomOut: () => controller?.zoomOut(),
    resetZoom: () => controller?.resetZoom(),
    fitToView: () => controller?.fitToView(),
  };
}
