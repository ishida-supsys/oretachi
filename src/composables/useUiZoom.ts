import { ref, readonly } from "vue";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { uiZoomFactor } from "../utils/uiScale";

// このウィンドウの webview に実際に適用されたズーム倍率。
// setZoom 失敗時 (例: 旧macOS) は設定値と乖離するため、フォント補正や
// CSS px→論理px 換算は設定値でなく必ずこちらを参照する。
const appliedZoom = ref(1.0);

/**
 * uiScale 設定をこのウィンドウの webview ズームに適用する (冪等)。
 * 成功時のみ appliedZoom を更新する。
 */
export async function applyUiZoom(settings: { appearance?: { uiScale?: string | null } | null }): Promise<void> {
  const zoom = uiZoomFactor(settings);
  if (zoom === appliedZoom.value) return;
  try {
    await getCurrentWebview().setZoom(zoom);
    appliedZoom.value = zoom;
  } catch (e) {
    console.warn("Failed to set webview zoom:", e);
  }
}

export function useUiZoom() {
  return { appliedZoom: readonly(appliedZoom) };
}
