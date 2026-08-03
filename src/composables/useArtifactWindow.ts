import { WebviewWindow } from "@tauri-apps/api/webviewWindow";

// キーは worktreeId、リポジトリスコープは `repo:<repositoryId>` の形で同じマップを共用する
const artifactWindowMap = new Map<string, WebviewWindow>();

/**
 * 既に開いているビューアがあればフォーカスして true を返す。
 * このマップはウィンドウごとのモジュール状態なので、呼び出し元ウィンドウがリロードされると
 * 空になる。その状態で同じラベルの WebviewWindow を作ると重複エラーになり、既存ウィンドウに
 * フォーカスも当たらず「ボタンが無反応」に見えるため、OS 側にも問い合わせる。
 */
async function focusExisting(key: string, label: string): Promise<boolean> {
  const cached = artifactWindowMap.get(key);
  if (cached) {
    await cached.setFocus();
    return true;
  }
  try {
    const found = await WebviewWindow.getByLabel(label);
    if (found) {
      artifactWindowMap.set(key, found);
      found.once("tauri://destroyed", () => {
        artifactWindowMap.delete(key);
      });
      await found.setFocus();
      return true;
    }
  } catch {
    /* 取得できなければ新規作成にフォールバックする */
  }
  return false;
}

/** リポジトリ ID（絶対パス）をウィンドウラベルに使える形へ変換する */
function encodeRepositoryLabel(repositoryId: string): string {
  // btoa は Latin-1 しか扱えないため、UTF-8 バイト列に落としてからエンコードする
  const bytes = new TextEncoder().encode(repositoryId);
  let binary = "";
  for (const b of bytes) binary += String.fromCharCode(b);
  return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=/g, "");
}

export function useArtifactWindow() {
  async function openArtifactViewer(
    worktreeId: string,
    worktreeName: string,
    repositoryName?: string,
  ): Promise<void> {
    if (await focusExisting(worktreeId, `artifact-${worktreeId}`)) return;

    const baseUrl = window.location.origin + window.location.pathname;
    const url =
      `${baseUrl}?mode=artifact&scope=worktree&worktreeId=${encodeURIComponent(worktreeId)}` +
      `&worktreeName=${encodeURIComponent(worktreeName)}` +
      (repositoryName ? `&repositoryName=${encodeURIComponent(repositoryName)}` : "");

    const win = new WebviewWindow(`artifact-${worktreeId}`, {
      url,
      title: `Artifacts - ${worktreeName}`,
      width: 900,
      height: 700,
      resizable: true,
      dragDropEnabled: false,
      transparent: true,
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

  /** リポジトリへ転送済み（恒久保存）のアーティファクトビューアを開く */
  async function openRepositoryArtifactViewer(
    repositoryId: string,
    repositoryName: string,
  ): Promise<void> {
    const key = `repo:${repositoryId}`;
    const label = `artifact-repo-${encodeRepositoryLabel(repositoryId)}`;
    if (await focusExisting(key, label)) return;

    const baseUrl = window.location.origin + window.location.pathname;
    const url =
      `${baseUrl}?mode=artifact&scope=repository&repositoryId=${encodeURIComponent(repositoryId)}` +
      `&repositoryName=${encodeURIComponent(repositoryName)}`;

    const win = new WebviewWindow(label, {
      url,
      title: `Artifacts - ${repositoryName}`,
      width: 900,
      height: 700,
      resizable: true,
      dragDropEnabled: false,
      transparent: true,
    });

    win.once("tauri://error", (e) => {
      console.error(`アーティファクトウィンドウ作成失敗: ${label}`, e);
      artifactWindowMap.delete(key);
    });

    win.once("tauri://destroyed", () => {
      artifactWindowMap.delete(key);
    });

    artifactWindowMap.set(key, win);
  }

  async function closeArtifactWindow(worktreeId: string): Promise<void> {
    const win = artifactWindowMap.get(worktreeId);
    if (win) {
      try { await win.destroy(); } catch { /* 既に閉じ済み */ }
      artifactWindowMap.delete(worktreeId);
    }
  }

  async function closeAllArtifactWindows(): Promise<void> {
    for (const [, win] of artifactWindowMap) {
      try { await win.destroy(); } catch { /* 既に閉じ済み */ }
    }
    artifactWindowMap.clear();
  }

  return {
    openArtifactViewer,
    openRepositoryArtifactViewer,
    closeArtifactWindow,
    closeAllArtifactWindows,
  };
}
