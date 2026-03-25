import { ref, computed, isRef, watch, onMounted, onUnmounted, nextTick } from "vue";
import type { Ref } from "vue";

interface MasonryOptions {
  minColumnWidth?: number | Ref<number>;
  gap?: number;
}

export function useMasonryLayout<T>(
  items: Ref<T[]>,
  options: MasonryOptions = {}
) {
  const { gap = 12 } = options;
  const minColWidth: Ref<number> = isRef(options.minColumnWidth)
    ? options.minColumnWidth
    : ref(options.minColumnWidth ?? 300);

  const containerRef = ref<HTMLElement | null>(null);
  const containerWidth = ref(0);

  const columnCount = computed(() =>
    Math.max(1, Math.floor((containerWidth.value + gap) / (minColWidth.value + gap)))
  );

  const columns = computed<T[][]>(() => {
    const cols: T[][] = Array.from({ length: columnCount.value }, () => []);
    items.value.forEach((item, i) => {
      cols[i % columnCount.value].push(item);
    });
    return cols;
  });

  let observer: ResizeObserver | null = null;

  function connect(el: HTMLElement) {
    const w = el.clientWidth;
    if (w > 0) containerWidth.value = w;
    observer = new ResizeObserver((entries) => {
      const width = entries[0].contentRect.width;
      if (width > 0) containerWidth.value = width;
    });
    observer.observe(el);
  }

  onMounted(() => {
    if (containerRef.value) {
      connect(containerRef.value);
      // WebView2 の初期レンダリングで末尾カードが描画されないバグを回避:
      // containerWidth が 0→実幅 に変わる2段階レンダリング後に強制 repaint を促す
      nextTick(() => {
        if (containerRef.value) {
          containerWidth.value = containerRef.value.clientWidth;
        }
      });
    }
  });

  watch(containerRef, (el) => {
    observer?.disconnect();
    observer = null;
    if (el) connect(el);
  });

  onUnmounted(() => {
    observer?.disconnect();
  });

  return { containerRef, columns };
}
