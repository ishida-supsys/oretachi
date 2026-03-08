import { WebviewWindow } from "@tauri-apps/api/webviewWindow";

const codeReviewWindowMap = new Map<string, WebviewWindow>();

export function useCodeReviewWindow() {
  async function openCodeReview(
    worktreeId: string,
    worktreeName: string,
    worktreePath: string,
  ): Promise<void> {
    const existing = codeReviewWindowMap.get(worktreeId);
    if (existing) {
      await existing.setFocus();
      return;
    }

    const baseUrl = window.location.origin + window.location.pathname;
    const url = `${baseUrl}?mode=codereview&worktreeId=${encodeURIComponent(worktreeId)}&worktreeName=${encodeURIComponent(worktreeName)}&worktreePath=${encodeURIComponent(worktreePath)}`;

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

    win.onCloseRequested(() => {
      codeReviewWindowMap.delete(worktreeId);
    });

    codeReviewWindowMap.set(worktreeId, win);
  }

  return { openCodeReview };
}
