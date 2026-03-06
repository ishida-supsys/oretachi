import { reactive } from "vue";
import { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import { emitTo, listen } from "@tauri-apps/api/event";

export interface SubWindowTerminal {
  id: number;
  title: string;
  sessionId: number;
  snapshot: string;
}

interface PendingInitData {
  terminals: SubWindowTerminal[];
  autoApproval: boolean;
}

const detachedWorktrees = reactive(new Set<string>());
const subWindowMap = new Map<string, WebviewWindow>();
const pendingInitData = new Map<string, PendingInitData>();

export function useSubWindows() {
  function isDetached(worktreeId: string): boolean {
    return detachedWorktrees.has(worktreeId);
  }

  async function moveToSubWindow(
    worktreeId: string,
    worktreeName: string,
    terminals: SubWindowTerminal[],
    autoApproval: boolean = false,
    restoring: boolean = false,
    worktreePath: string = "",
  ): Promise<void> {
    if (detachedWorktrees.has(worktreeId)) {
      const win = subWindowMap.get(worktreeId);
      if (win) await win.setFocus();
      return;
    }

    // window.location.origin + pathname を使い dev/prod 両方で正しい URL を構築
    const baseUrl = window.location.origin + window.location.pathname;
    const url = `${baseUrl}?mode=subwindow&worktreeId=${encodeURIComponent(worktreeId)}&worktreeName=${encodeURIComponent(worktreeName)}&worktreePath=${encodeURIComponent(worktreePath)}`;

    const win = new WebviewWindow(`sub-${worktreeId}`, {
      url,
      title: `oretachi - ${worktreeName}`,
      width: 900,
      height: 600,
      visible: restoring ? false : undefined,
      resizable: true,
      dragDropEnabled: false,  // HTML5 D&D を有効にするために必要 (Windows)
    });

    win.once("tauri://created", () => {
      console.log(`サブウィンドウ作成成功: sub-${worktreeId}`);
    });

    win.once("tauri://error", (e) => {
      console.error(`サブウィンドウ作成失敗: sub-${worktreeId}`, e);
      pendingInitData.delete(worktreeId);
      subWindowMap.delete(worktreeId);
      detachedWorktrees.delete(worktreeId);
    });

    // ターミナルデータはサブウィンドウ準備完了後にイベントで送る
    pendingInitData.set(worktreeId, { terminals, autoApproval });

    subWindowMap.set(worktreeId, win);
    detachedWorktrees.add(worktreeId);
  }

  async function moveToMainWindow(worktreeId: string): Promise<void> {
    const win = subWindowMap.get(worktreeId);
    if (win) {
      try {
        // ack リスナーを先にセットアップ（レースコンディション防止）
        const ackPromise = new Promise<void>((resolve) => {
          const timeout = setTimeout(() => {
            unlisten();
            resolve();
          }, 5000);

          let unlisten = () => {};
          listen<{ worktreeId: string }>("sub-window-closed-ack", (event) => {
            if (event.payload.worktreeId === worktreeId) {
              clearTimeout(timeout);
              unlisten();
              resolve();
            }
          }).then((fn) => { unlisten = fn; });
        });

        await emitTo(`sub-${worktreeId}`, "sub-closing-by-main", { worktreeId });
        await ackPromise;
      } catch {
        // ウィンドウが既に閉じられている場合は無視
      }
      subWindowMap.delete(worktreeId);
    }
    pendingInitData.delete(worktreeId);
    detachedWorktrees.delete(worktreeId);
  }

  async function focusSubWindow(worktreeId: string): Promise<void> {
    const win = subWindowMap.get(worktreeId);
    if (win) {
      await win.setFocus();
    }
  }

  function unregisterSubWindow(worktreeId: string): void {
    subWindowMap.delete(worktreeId);
    pendingInitData.delete(worktreeId);
    detachedWorktrees.delete(worktreeId);
  }

  function getPendingInitData(worktreeId: string): PendingInitData | undefined {
    return pendingInitData.get(worktreeId);
  }

  function clearPendingInitData(worktreeId: string): void {
    pendingInitData.delete(worktreeId);
  }

  async function closeAllSubWindows(): Promise<void> {
    const worktreeIds = Array.from(subWindowMap.keys());
    if (worktreeIds.length === 0) return;

    const closePromises = worktreeIds.map((worktreeId) => {
      return new Promise<void>((resolve) => {
        const timeout = setTimeout(() => {
          unlisten();
          resolve();
        }, 5000);

        let unlisten = () => {};
        listen<{ worktreeId: string }>("sub-window-closed-ack", (event) => {
          if (event.payload.worktreeId === worktreeId) {
            clearTimeout(timeout);
            unlisten();
            resolve();
          }
        }).then((fn) => { unlisten = fn; });

        emitTo(`sub-${worktreeId}`, "sub-closing-by-main", { worktreeId }).catch(() => {
          clearTimeout(timeout);
          unlisten();
          resolve();
        });
      });
    });

    await Promise.all(closePromises);

    subWindowMap.clear();
    pendingInitData.clear();
    detachedWorktrees.clear();
  }

  return {
    detachedWorktrees,
    isDetached,
    moveToSubWindow,
    moveToMainWindow,
    focusSubWindow,
    unregisterSubWindow,
    getPendingInitData,
    clearPendingInitData,
    closeAllSubWindows,
  };
}
