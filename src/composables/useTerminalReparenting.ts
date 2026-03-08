import { markRaw } from "vue";

interface HasContainerRef {
  containerRef: HTMLElement | null;
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
  }

  /** 各TerminalViewを対応するterminal-hostに移動 */
  function mountTerminalsToHosts(): void {
    for (const [tid] of terminalEntries) {
      const comp = terminalRefs.get(tid);
      const el = comp?.containerRef;
      const host = document.getElementById(`terminal-host-${tid}`);
      if (el && host && el.parentElement !== host) {
        host.appendChild(el);
      }
    }
  }

  return { setTerminalRef, returnAllToOffscreen, mountTerminalsToHosts };
}
