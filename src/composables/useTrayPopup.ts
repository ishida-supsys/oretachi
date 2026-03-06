import { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import type { FrameNode } from "../types/frame";

export interface TrayTerminalData {
  id: number;
  title: string;
  sessionId: number;
  snapshot: string;
  rows: number;
  cols: number;
}

export interface TrayWorktreeData {
  worktreeId: string;
  worktreeName: string;
  isDetached: boolean;
  layout: FrameNode | null;
  terminals: TrayTerminalData[];
  windowSize?: { width: number; height: number };
}

let trayWindow: WebviewWindow | null = null;

export function useTrayPopup() {
  async function openTrayPopup(worktrees: TrayWorktreeData[]): Promise<void> {
    if (trayWindow) {
      try {
        await trayWindow.setFocus();
      } catch {
        trayWindow = null;
      }
      if (trayWindow) return;
    }

    const baseUrl = window.location.origin + window.location.pathname;
    const url = `${baseUrl}?mode=tray`;

    trayWindow = new WebviewWindow("tray-popup", {
      url,
      title: "oretachi - 通知",
      width: 900,
      height: 600,
      resizable: true,
      decorations: false,
      dragDropEnabled: false,
    });

    // ペンディングデータを保持（tray-ready 受信後に main が送信する）
    // worktrees データは App.vue 側が tray-ready → tray-init で送信する
    // ここでは参照のために pendingWorktrees に保持しておく
    pendingWorktrees = worktrees;

    trayWindow.once("tauri://error", (e) => {
      console.error("トレイポップアップ作成失敗:", e);
      trayWindow = null;
      pendingWorktrees = null;
    });
  }

  function closeTrayPopup(): void {
    if (trayWindow) {
      trayWindow.destroy().catch(() => {});
      trayWindow = null;
    }
    pendingWorktrees = null;
  }

  function getPendingWorktrees(): TrayWorktreeData[] | null {
    return pendingWorktrees;
  }

  function clearPendingWorktrees(): void {
    pendingWorktrees = null;
  }

  return {
    openTrayPopup,
    closeTrayPopup,
    getPendingWorktrees,
    clearPendingWorktrees,
  };
}

// モジュールスコープで pendingWorktrees を保持
let pendingWorktrees: TrayWorktreeData[] | null = null;
