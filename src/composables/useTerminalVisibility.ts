import type { Terminal } from "@xterm/xterm";
import { WebglAddon } from "@xterm/addon-webgl";
import { logDebug } from "../utils/log";

// WebGL コンテキストはブラウザ全体で上限 (~16) があり、端末数ぶん生成すると
// コンテキストロスの連鎖や compositor 停止 (webview ハング) を招く。
// 可視端末のみロードする方針に加え、保険として同時ロード数に上限を設ける。
// カウンタはモジュールスコープ = 同一 webview 内の全端末で共有。
const MAX_WEBGL_CONTEXTS = 8;
let webglContextCount = 0;

/** オフスクリーン退避後に WebGL を破棄するまでの猶予 (トレイ開閉連打での生成/破棄チャーン回避)。 */
const WEBGL_DISPOSE_DELAY_MS = 5000;

interface TerminalVisibilityOptions {
  getTerminal: () => Terminal | null;
  /** オフスクリーン div ([data-offscreen]) 配下にいるか */
  isOffscreen: () => boolean;
  /** オフスクリーン時の write 抑制切替 (usePtyWriteBatcher.setSuspended) */
  setWriteSuspended: (suspended: boolean) => void;
  /** ログ用 */
  getSessionId: () => number | null;
}

/**
 * ターミナルのオフスクリーン ↔ 可視 状態に応じたリソース管理。
 *
 * - 可視: WebGL をロード (上限内) し、write 抑制を解除して蓄積分を排出。
 * - オフスクリーン: write を抑制し、猶予後に WebGL を破棄。
 *
 * updateVisibility は DOM reparenting (useTerminalReparenting) と
 * handleTabActivated から呼ばれる。dispose は onUnmounted で呼ぶこと
 * (WebGL カウンタの整合のため terminal.dispose() より前)。
 */
export function useTerminalVisibility(options: TerminalVisibilityOptions) {
  const { getTerminal, isOffscreen, setWriteSuspended, getSessionId } = options;

  let webglAddon: WebglAddon | null = null;
  let disposeTimer: ReturnType<typeof setTimeout> | null = null;

  function loadWebgl(): void {
    const terminal = getTerminal();
    if (webglAddon || !terminal) return;
    if (webglContextCount >= MAX_WEBGL_CONTEXTS) {
      logDebug(`[Terminal] WebGL load skipped (limit ${MAX_WEBGL_CONTEXTS}) sid=${getSessionId()}`);
      return;
    }
    try {
      const addon = new WebglAddon();
      addon.onContextLoss(() => {
        console.warn("[XTERM] WebGL onContextLoss fired!", { sessionId: getSessionId() });
        disposeWebgl();
        // dispose 後は xterm.js が自動で DOM レンダラーにフォールバック。
        // 明示的に refresh を呼んで再描画させる。
        const t = getTerminal();
        t?.refresh(0, t.rows - 1);
      });
      terminal.loadAddon(addon);
      webglAddon = addon;
      webglContextCount++;
      logDebug(`[Terminal] WebGL loaded sid=${getSessionId()} count=${webglContextCount}`);
    } catch {
      // DOM renderer を使用
    }
  }

  function disposeWebgl(): void {
    if (!webglAddon) return;
    try {
      webglAddon.dispose();
    } catch {
      // すでに破棄済み等は無視
    }
    webglAddon = null;
    webglContextCount--;
    logDebug(`[Terminal] WebGL disposed sid=${getSessionId()} count=${webglContextCount}`);
  }

  function clearDisposeTimer(): void {
    if (disposeTimer !== null) {
      clearTimeout(disposeTimer);
      disposeTimer = null;
    }
  }

  /** オフスクリーン ↔ 可視 の状態変化を反映する。 */
  function updateVisibility(): void {
    const offscreen = isOffscreen();
    setWriteSuspended(offscreen);
    if (offscreen) {
      if (webglAddon && disposeTimer === null) {
        disposeTimer = setTimeout(() => {
          disposeTimer = null;
          // 猶予中に再可視化されていたら破棄しない
          if (isOffscreen()) {
            disposeWebgl();
            const t = getTerminal();
            t?.refresh(0, t.rows - 1);
          }
        }, WEBGL_DISPOSE_DELAY_MS);
      }
    } else {
      clearDisposeTimer();
      loadWebgl();
    }
  }

  function dispose(): void {
    clearDisposeTimer();
    disposeWebgl();
  }

  return { updateVisibility, dispose };
}
