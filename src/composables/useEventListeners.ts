import { onUnmounted } from "vue";
import type { UnlistenFn } from "@tauri-apps/api/event";

/**
 * listen() の戻り値（UnlistenFn）を収集し、onUnmounted で一括解除するヘルパー。
 * collect(await listen(...)) のように使う。
 */
export function useEventListeners() {
  const fns: UnlistenFn[] = [];

  function collect(fn: UnlistenFn): void {
    fns.push(fn);
  }

  onUnmounted(() => {
    for (const fn of fns) fn();
    fns.length = 0;
  });

  return { collect };
}
