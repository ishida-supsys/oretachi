import { WebviewWindow } from "@tauri-apps/api/webviewWindow";

const artifactWindowMap = new Map<string, WebviewWindow>();

export function useArtifactWindow() {
  async function openArtifactViewer(
    worktreeId: string,
    worktreeName: string,
  ): Promise<void> {
    const existing = artifactWindowMap.get(worktreeId);
    if (existing) {
      await existing.setFocus();
      return;
    }

    const baseUrl = window.location.origin + window.location.pathname;
    const url = `${baseUrl}?mode=artifact&worktreeId=${encodeURIComponent(worktreeId)}&worktreeName=${encodeURIComponent(worktreeName)}`;

    const win = new WebviewWindow(`artifact-${worktreeId}`, {
      url,
      title: `Artifacts - ${worktreeName}`,
      width: 900,
      height: 700,
      resizable: true,
      dragDropEnabled: false,
      transparent: true,
    });

    win.once("tauri://created", () => {
      console.log(`アーティファクトウィンドウ作成成功: artifact-${worktreeId}`);
    });

    win.once("tauri://error", (e) => {
      console.error(`アーティファクトウィンドウ作成失敗: artifact-${worktreeId}`, e);
      artifactWindowMap.delete(worktreeId);
    });

    win.once("tauri://destroyed", () => {
      artifactWindowMap.delete(worktreeId);
    });

    artifactWindowMap.set(worktreeId, win);
  }

  async function closeAllArtifactWindows(): Promise<void> {
    for (const [, win] of artifactWindowMap) {
      try { await win.destroy(); } catch { /* 既に閉じ済み */ }
    }
    artifactWindowMap.clear();
  }

  return { openArtifactViewer, closeAllArtifactWindows };
}
