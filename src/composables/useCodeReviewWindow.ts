import { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import { invoke } from "@tauri-apps/api/core";
import type { CodeReviewOrigin } from "./useCodeReviewLineChat";

const codeReviewWindowMap = new Map<string, WebviewWindow>();

export function useCodeReviewWindow() {
  async function openCodeReview(
    worktreeId: string,
    worktreeName: string,
    worktreePath: string,
    origin: CodeReviewOrigin = "main",
  ): Promise<void> {
    const existing = codeReviewWindowMap.get(worktreeId);
    if (existing) {
      await existing.setFocus();
      return;
    }

    const baseUrl = window.location.origin + window.location.pathname;
    const url = `${baseUrl}?mode=codereview&worktreeId=${encodeURIComponent(worktreeId)}&worktreeName=${encodeURIComponent(worktreeName)}&worktreePath=${encodeURIComponent(worktreePath)}&origin=${encodeURIComponent(origin)}`;

    const win = new WebviewWindow(`codereview-${worktreeId}`, {
      url,
      title: `Code Review - ${worktreeName}`,
      width: 1200,
      height: 800,
      resizable: true,
      dragDropEnabled: false,
    });

    win.once("tauri://created", () => {
      console.log(`コードレビューウィンドウ作成成功: codereview-${worktreeId}`);
    });

    win.once("tauri://error", (e) => {
      console.error(`コードレビューウィンドウ作成失敗: codereview-${worktreeId}`, e);
      codeReviewWindowMap.delete(worktreeId);
    });

    win.once("tauri://destroyed", () => {
      codeReviewWindowMap.delete(worktreeId);
      invoke("stop_fs_watch", { worktreeId }).catch(() => {});
    });

    codeReviewWindowMap.set(worktreeId, win);
  }

  async function closeAllCodeReviewWindows(): Promise<void> {
    for (const [, win] of codeReviewWindowMap) {
      try { await win.destroy(); } catch { /* 既に閉じ済み */ }
    }
    codeReviewWindowMap.clear();
  }

  return { openCodeReview, closeAllCodeReviewWindows };
}
