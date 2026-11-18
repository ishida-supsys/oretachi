import { ref, onMounted, onUnmounted } from "vue";
import { getCurrentWindow } from "@tauri-apps/api/window";

/**
 * ウィンドウが「完全に隠れている」状態かを返す。
 * 最小化・非表示(トレイへ隠した状態)・document.visibilityState === "hidden" のいずれか。
 * 無限アニメーションや定期描画を止める判断に使う。
 *
 * イベント(リサイズ・フォーカス・visibilitychange)だけでは取りこぼす。フォーカスなしの
 * show / 非フォーカスでのトレイ非表示ではどれも発火せず、状態が逆のまま固まりうる。
 * そのため低頻度(2.5 秒)で再評価も行う。getter 2 回の IPC だけで描画は伴わない。
 */
const REEVALUATE_INTERVAL_MS = 2500;

export function useWindowHidden() {
  const isWindowHidden = ref(false);
  let disposed = false;
  const unlisteners: Array<() => void> = [];
  let timer: ReturnType<typeof setInterval> | null = null;
  // 非同期の評価が前後して終わっても、最後に始めた評価の結果だけを反映する
  let generation = 0;

  async function evaluate() {
    const my = ++generation;
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
    if (!disposed && my === generation) isWindowHidden.value = hidden;
  }

  function onVisibilityChange() {
    void evaluate();
  }

  onMounted(() => {
    void evaluate();
    document.addEventListener("visibilitychange", onVisibilityChange);
    timer = setInterval(() => void evaluate(), REEVALUATE_INTERVAL_MS);
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
    if (timer) { clearInterval(timer); timer = null; }
    document.removeEventListener("visibilitychange", onVisibilityChange);
    unlisteners.forEach((un) => un());
    unlisteners.length = 0;
  });

  return { isWindowHidden };
}
