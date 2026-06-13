import { ref, onMounted, onUnmounted } from "vue";

/**
 * ループアニメーション用 tick。0 → total-1 を intervalMs 間隔で周回する。
 * setup() 内で呼ぶこと (onMounted/onUnmounted でタイマーを管理する)。
 */
export function useLoopTick(total: number, intervalMs: number) {
  const tick = ref(0);
  let timer: ReturnType<typeof setInterval> | null = null;

  onMounted(() => {
    timer = setInterval(() => {
      tick.value = (tick.value + 1) % total;
    }, intervalMs);
  });

  onUnmounted(() => {
    if (timer) clearInterval(timer);
  });

  return tick;
}
