import { ref, computed, watch } from "vue";
import type { Reactive } from "vue";
import { useTasks } from "./useTasks";

export function useShutdownGuard(loadingWorktrees: Reactive<Map<string, string>>) {
  const { tasks } = useTasks();

  const isWaitingForShutdown = ref(false);

  const isBusyForShutdown = computed(() => {
    if (loadingWorktrees.size > 0) return true;
    return tasks.value.some(
      (t) => t.status === "generating" || t.status === "executing"
    );
  });

  function waitForBusyOperations(timeoutMs = 20000): Promise<void> {
    return new Promise((resolve) => {
      if (!isBusyForShutdown.value) { resolve(); return; }
      let stop: () => void;
      const timer = setTimeout(() => { stop(); resolve(); }, timeoutMs);
      stop = watch(isBusyForShutdown, (busy) => {
        if (!busy) { clearTimeout(timer); stop(); resolve(); }
      });
    });
  }

  return { isWaitingForShutdown, isBusyForShutdown, waitForBusyOperations };
}
