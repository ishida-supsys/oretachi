import { ref, onMounted, onUnmounted } from "vue";
import { getCurrentWindow } from "@tauri-apps/api/window";

/**
 * ウィンドウが「完全に隠れている」状態かを返す。
 * 最小化・非表示(トレイへ隠した状態)・document.visibilityState === "hidden" のいずれか。
 * 無限アニメーションや定期描画を止める判断に使う。ポーリングはせずイベントで再評価する。
 */
export function useWindowHidden() {
  const isWindowHidden = ref(false);
  let disposed = false;
  const unlisteners: Array<() => void> = [];

  async function evaluate() {
    let hidden = document.visibilityState === "hidden";
    if (!hidden) {
      try {
        const win = getCurrentWindow();
        const [minimized, visible] = await Promise.all([win.isMinimized(), win.isVisible()]);
        hidden = minimized || !visible;
      } catch {
        // ウィンドウ API が使えない環境では visibilityState だけで判定する
      }
    }
    if (!disposed) isWindowHidden.value = hidden;
  }

  function onVisibilityChange() {
    void evaluate();
  }

  onMounted(() => {
    void evaluate();
    document.addEventListener("visibilitychange", onVisibilityChange);
    const win = getCurrentWindow();
    // 最小化・復元はリサイズ、hide/show はフォーカス変化を伴う
    for (const p of [win.onResized(() => void evaluate()), win.onFocusChanged(() => void evaluate())]) {
      p.then((un) => {
        if (disposed) un();
        else unlisteners.push(un);
      }).catch(() => {});
    }
  });

  onUnmounted(() => {
    disposed = true;
    document.removeEventListener("visibilitychange", onVisibilityChange);
    unlisteners.forEach((un) => un());
    unlisteners.length = 0;
  });

  return { isWindowHidden };
}
