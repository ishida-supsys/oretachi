import { markRaw } from "vue";
import { logDebug } from "../utils/log";

interface HasContainerRef {
  containerRef: HTMLElement | null;
  /** オフスクリーン ↔ 可視 の状態変化を反映する (TerminalView が公開)。 */
  updateVisibility?: () => void;
}

/**
 * TerminalView の DOM 要素をオフスクリーン ↔ terminal-host 間で移動するユーティリティ。
 * SubWindowApp / TrayPopupApp で共通利用する。
 */
export function useTerminalReparenting<TEntry, TRef extends HasContainerRef>(
  terminalEntries: Map<number, TEntry>,
  terminalRefs: Map<number, TRef>
) {
  function setTerminalRef(terminalId: number, el: unknown): void {
    if (el) {
      terminalRefs.set(terminalId, markRaw(el as TRef));
    } else {
      terminalRefs.delete(terminalId);
    }
  }

  /** DOM 移動後に各 TerminalView へ可視状態の変化を通知する (WebGL/write 抑制の切替)。 */
  function notifyVisibility(): void {
    for (const [tid] of terminalEntries) {
      terminalRefs.get(tid)?.updateVisibility?.();
    }
  }

  /** 全TerminalViewをオフスクリーンdivに退避 */
  function returnAllToOffscreen(): void {
    const offscreen = document.querySelector("[data-offscreen]");
    if (!offscreen) return;
    for (const [tid] of terminalEntries) {
      const comp = terminalRefs.get(tid);
      const el = comp?.containerRef;
      if (el && el.parentElement !== offscreen) {
        offscreen.appendChild(el);
      }
    }
    notifyVisibility();
  }

  /** 各TerminalViewを対応するterminal-hostに移動 */
  function mountTerminalsToHosts(): void {
    for (const [tid] of terminalEntries) {
      const comp = terminalRefs.get(tid);
      const el = comp?.containerRef;
      const host = document.getElementById(`terminal-host-${tid}`);
      // 注意: ここで host.clientWidth 等を読むと appendChild (レイアウト無効化) と
      // 交互になり端末数ぶん強制同期リフローが発生する (layout thrashing)。読まないこと。
      logDebug(
        `[Terminal] mountTerminalsToHosts tid=${tid} host=${host ? "found" : "not_found"} el=${el ? "found" : "not_found"}`
      );
      if (el && host && el.parentElement !== host) {
        host.appendChild(el);
      }
    }
    notifyVisibility();
  }

  return { setTerminalRef, returnAllToOffscreen, mountTerminalsToHosts };
}
