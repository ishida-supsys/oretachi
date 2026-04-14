import { warn } from "@tauri-apps/plugin-log";

const INTERVAL_MS = 2000;
const BLOCK_THRESHOLD_MS = 5000;

let intervalId: ReturnType<typeof setInterval> | null = null;

/**
 * JS メインスレッドのイベントループ遅延を監視する。
 * setInterval の実測間隔が期待値を大幅に超えた場合、メインスレッドがブロックされていたと判定してログに記録する。
 */
export function startEventLoopMonitor(): void {
  if (intervalId !== null) return;
  let lastTs = performance.now();
  intervalId = setInterval(() => {
    const now = performance.now();
    const delta = now - lastTs;
    const blocked = delta - INTERVAL_MS;
    if (blocked >= BLOCK_THRESHOLD_MS) {
      warn(`[EventLoop] main thread blocked for ${Math.round(blocked)}ms`);
    }
    lastTs = now;
  }, INTERVAL_MS);
}
