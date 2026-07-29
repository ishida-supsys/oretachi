/**
 * パン (ドラッグ移動) とズーム (ホイール拡大縮小) のコア実装。
 *
 * Vue に依存しない DOM ユーティリティとして持つことで、
 * - アーティファクトの全画面ビュー (usePanZoom → PanZoomCanvas)
 * - markdown プレビュー内 mermaid のインラインズーム (ArtifactMarkdownView)
 * の双方で同じ操作感 (ズームの刻み・カーソル基準・ドラッグ閾値) を共有する。
 *
 * transform の適用は行わず onChange で状態を通知するだけなので、
 * Vue 側は宣言的なスタイルバインド、DOM 側は style 直書きとそれぞれの流儀で反映できる。
 */

export interface PanZoomState {
  x: number;
  y: number;
  zoom: number;
  dragging: boolean;
}

export interface PanZoomOptions {
  /** 最小ズーム倍率 */
  min?: number;
  /** 最大ズーム倍率 */
  max?: number;
  /** フィット時にコンテナ内側に残す余白(px) */
  padding?: number;
  /** フィット時にこれ以上は拡大しない上限 (小さな図が過剰に巨大化するのを防ぐ) */
  maxFitZoom?: number;
  /** 状態が変わるたびに呼ばれる */
  onChange: (state: PanZoomState) => void;
}

export interface PanZoomController {
  zoomIn(): void;
  zoomOut(): void;
  /** 100% に戻して中央寄せする */
  resetZoom(): void;
  /** コンテンツ全体がコンテナに収まる倍率に合わせて中央寄せする */
  fitToView(): void;
  destroy(): void;
}

/** ドラッグ開始とみなす移動量(px)。これ未満はクリック扱いにする */
const DRAG_THRESHOLD = 3;

/** ホイール 1 ノッチあたりの倍率。加算ではなく乗算にして操作量に対する体感を揃える */
const WHEEL_FACTOR = 1.1;

/** 1 ノッチとみなす deltaY(px)。マウスホイールの 1 段が概ねこの値 */
const WHEEL_NOTCH_PX = 100;

/** deltaMode (0=PIXEL, 1=LINE, 2=PAGE) を px 換算する係数 */
const DELTA_MODE_TO_PX = [1, 16, 800];

/** 1 イベントで動かす上限ノッチ数。deltaMode=PAGE などの極端な値で飛びすぎるのを防ぐ */
const MAX_NOTCHES_PER_EVENT = 3;

/**
 * container 上の操作を監視して、target のパン/ズーム状態を管理する。
 *
 * @param container ビューポートとなる要素 (イベントを受ける)
 * @param getTarget 変形対象の要素を返す関数 (フィット時のサイズ計測に使う)
 */
export function createPanZoom(
  container: HTMLElement,
  getTarget: () => HTMLElement | SVGElement | null,
  options: PanZoomOptions
): PanZoomController {
  const min = options.min ?? 0.15;
  const max = options.max ?? 5;
  const padding = options.padding ?? 24;
  const maxFitZoom = options.maxFitZoom ?? 2;

  let x = 0;
  let y = 0;
  let zoom = 1;
  let dragging = false;

  const notify = () => options.onChange({ x, y, zoom, dragging });

  const clampZoom = (z: number) => Math.min(max, Math.max(min, z));

  /** コンテナ座標 (cx, cy) を固定点として factor 倍ズームする */
  function zoomAt(cx: number, cy: number, factor: number) {
    const next = clampZoom(zoom * factor);
    const ratio = next / zoom;
    if (ratio === 1) return;
    x = cx - (cx - x) * ratio;
    y = cy - (cy - y) * ratio;
    zoom = next;
    notify();
  }

  function zoomAtCenter(factor: number) {
    zoomAt(container.clientWidth / 2, container.clientHeight / 2, factor);
  }

  /** transform の影響を受けないレイアウトサイズを取る */
  function naturalSize(): { w: number; h: number } | null {
    const el = getTarget();
    if (!el) return null;
    const rect = (el as HTMLElement).offsetWidth
      ? { w: (el as HTMLElement).offsetWidth, h: (el as HTMLElement).offsetHeight }
      : { w: el.getBoundingClientRect().width / zoom, h: el.getBoundingClientRect().height / zoom };
    if (!rect.w || !rect.h) return null;
    return rect;
  }

  /** 指定ズーム(null ならフィット倍率)を適用してコンテンツをコンテナ中央に置く */
  function applyZoomCentered(targetZoom: number | null) {
    const size = naturalSize();
    if (!size) return;
    const vw = container.clientWidth;
    const vh = container.clientHeight;
    const z =
      targetZoom ??
      Math.min(
        maxFitZoom,
        clampZoom(Math.min((vw - padding * 2) / size.w, (vh - padding * 2) / size.h))
      );
    zoom = clampZoom(z);
    x = (vw - size.w * zoom) / 2;
    y = (vh - size.h * zoom) / 2;
    notify();
  }

  // --- ドラッグによるパン ---
  // ドラッグ中はカーソルがコンテナ外に出ても追従したいので window でイベントを受ける
  let lastPos: { x: number; y: number } | null = null;
  let dragMoved = false;

  function stopDrag() {
    lastPos = null;
    dragMoved = false;
    if (dragging) {
      dragging = false;
      notify();
    }
    window.removeEventListener("mousemove", onWindowMouseMove);
    window.removeEventListener("mouseup", stopDrag);
  }

  function onWindowMouseMove(e: MouseEvent) {
    if (!lastPos) return;
    const dx = e.clientX - lastPos.x;
    const dy = e.clientY - lastPos.y;
    if (!dragMoved) {
      if (Math.abs(dx) < DRAG_THRESHOLD && Math.abs(dy) < DRAG_THRESHOLD) return;
      dragMoved = true;
      dragging = true;
    }
    x += dx;
    y += dy;
    lastPos = { x: e.clientX, y: e.clientY };
    notify();
  }

  /** [data-ui] 要素上 (ツールバー等) では何もしない */
  function onMouseDown(e: MouseEvent) {
    if (e.button !== 0) return;
    const target = e.target as HTMLElement | null;
    if (target?.closest?.("[data-ui]")) return;
    e.preventDefault(); // ドラッグ中のテキスト選択を防ぐ
    lastPos = { x: e.clientX, y: e.clientY };
    dragMoved = false;
    window.addEventListener("mousemove", onWindowMouseMove);
    window.addEventListener("mouseup", stopDrag);
  }

  // --- ホイールによるズーム ---
  // ブラウザのスクロール/ズームを止めるため passive: false で登録する
  function onWheel(e: WheelEvent) {
    // 横スクロール (deltaX のみ) はズーム操作ではないので、スクロールを妨げずに見送る。
    // トラックパッドの2本指横スワイプやチルトホイールがここに来る
    if (e.deltaY === 0) return;
    e.preventDefault();
    // 倍率は deltaY の量に比例させる。1 イベント固定倍率にすると、ピクセル粒度で
    // 大量の wheel を送るトラックパッドでは 1 スワイプで最大倍率に張り付いてしまう
    const px = e.deltaY * (DELTA_MODE_TO_PX[e.deltaMode] ?? 1);
    const notches = Math.max(
      -MAX_NOTCHES_PER_EVENT,
      Math.min(MAX_NOTCHES_PER_EVENT, px / WHEEL_NOTCH_PX)
    );
    const rect = container.getBoundingClientRect();
    zoomAt(e.clientX - rect.left, e.clientY - rect.top, Math.pow(WHEEL_FACTOR, -notches));
  }

  container.addEventListener("wheel", onWheel, { passive: false });
  container.addEventListener("mousedown", onMouseDown);

  return {
    zoomIn: () => zoomAtCenter(1.2),
    zoomOut: () => zoomAtCenter(1 / 1.2),
    resetZoom: () => applyZoomCentered(1),
    fitToView: () => applyZoomCentered(null),
    destroy() {
      container.removeEventListener("wheel", onWheel);
      container.removeEventListener("mousedown", onMouseDown);
      stopDrag();
    },
  };
}
