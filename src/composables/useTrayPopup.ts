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
  worktreePath: string;
  isDetached: boolean;
  layout: FrameNode | null;
  terminals: TrayTerminalData[];
  windowSize?: { width: number; height: number };
  branchName: string;
  hotkeyChar?: string;
  autoApproval: boolean;
  aiJudging: boolean;
  autoApprovalPrompt?: string;
  lastJudgedCommand?: string;
}

let trayWindow: WebviewWindow | null = null;
let isOpening = false;

export function useTrayPopup() {
  async function openTrayPopup(worktrees: TrayWorktreeData[]): Promise<void> {
    if (isOpening) return;

    if (trayWindow) {
      try {
        await trayWindow.setFocus();
      } catch {
        trayWindow = null;
      }
      if (trayWindow) return;
    }

    isOpening = true;
    try {
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
        transparent: true,
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
    } finally {
      isOpening = false;
    }
  }

  async function closeTrayPopup(skipDestroy = false): Promise<void> {
    if (trayWindow && !skipDestroy) {
      await trayWindow.destroy().catch(() => {});
    }
    trayWindow = null;
    pendingWorktrees = null;
    currentTrayWorktreeId = null;
  }

  function getPendingWorktrees(): TrayWorktreeData[] | null {
    return pendingWorktrees;
  }

  function clearPendingWorktrees(): void {
    pendingWorktrees = null;
  }

  function setCurrentTrayWorktreeId(worktreeId: string | null): void {
    currentTrayWorktreeId = worktreeId;
  }

  function isTrayShowingWorktree(worktreeId: string): boolean {
    return trayWindow !== null && currentTrayWorktreeId === worktreeId;
  }

  async function focusTrayWindow(): Promise<void> {
    if (trayWindow) {
      await trayWindow.setFocus();
    }
  }

  return {
    openTrayPopup,
    closeTrayPopup,
    getPendingWorktrees,
    clearPendingWorktrees,
    setCurrentTrayWorktreeId,
    isTrayShowingWorktree,
    focusTrayWindow,
  };
}

// モジュールスコープで pendingWorktrees を保持
let pendingWorktrees: TrayWorktreeData[] | null = null;
// トレイポップアップが現在表示しているワークツリーID
let currentTrayWorktreeId: string | null = null;
