import { ref, computed, isRef, watch, onMounted, onUnmounted } from "vue";
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
    containerWidth.value = el.clientWidth;
    observer = new ResizeObserver((entries) => {
      containerWidth.value = entries[0].contentRect.width;
    });
    observer.observe(el);
  }

  onMounted(() => {
    if (containerRef.value) connect(containerRef.value);
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
