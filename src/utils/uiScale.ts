/**
 * UIスケール設定 (appearance.uiScale) のズーム換算ユーティリティ。
 *
 * 適用機構は webview ズーム (getCurrentWebview().setZoom) で、ブラウザズームと
 * 同じセマンティクス。ズーム Z 下では CSS px × Z = 論理px (DIP/LogicalSize)。
 */

/** 'large' 選択時のズーム倍率 (VSCode のズーム1段階相当) */
export const UI_SCALE_LARGE = 1.2;

/** 'xlarge' 選択時のズーム倍率 (VSCode のズーム2段階相当 = 1.2^2) */
export const UI_SCALE_XLARGE = 1.44;

/** uiScale 設定値からズーム倍率を返す (未知の値はすべて 1.0) */
export function uiZoomFactor(settings: { appearance?: { uiScale?: string | null } | null }): number {
  switch (settings.appearance?.uiScale) {
    case "large":
      return UI_SCALE_LARGE;
    case "xlarge":
      return UI_SCALE_XLARGE;
    default:
      return 1.0;
  }
}

/** webview 内の CSS px 計測値を LogicalSize 用の論理pxに換算する */
export function cssPxToLogical(px: number, zoom: number): number {
  return Math.round(px * zoom);
}
