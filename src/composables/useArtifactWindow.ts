import { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import { emitTo } from "@tauri-apps/api/event";

/** 既に開いているビューアへ「このアーティファクトを選べ」と指示する Tauri イベント */
export const ARTIFACT_NAVIGATE_EVENT = "artifact-navigate";

export interface ArtifactNavigateEvent {
  artifactId: string;
}

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

/**
 * 既存ウィンドウへの遷移指示。
 * 新規ウィンドウ側で同じことをイベントでやると `listen` 登録前に emit してしまい
 * 取りこぼすため、新規は URL の `artifactId` パラメータで渡すこと。
 */
async function requestNavigate(label: string, artifactId: string): Promise<void> {
  try {
    await emitTo(label, ARTIFACT_NAVIGATE_EVENT, { artifactId } satisfies ArtifactNavigateEvent);
  } catch (e) {
    console.error(`アーティファクト遷移指示の送信に失敗: ${label}`, e);
  }
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
  /**
   * ワークツリーのアーティファクトビューアを開く。
   * `artifactId` を渡すとそのアーティファクトを選択した状態で開く（既存ウィンドウなら遷移）。
   *
   * URL には ID しか載せない。リンクから他ワークツリーへ遷移する側は遷移先の名前を知らないため、
   * 名前はビューア起動後に `resolve_artifact_scope` で settings から解決する。
   */
  async function openArtifactViewer(
    worktreeId: string,
    artifactId?: string,
  ): Promise<void> {
    const label = `artifact-${worktreeId}`;
    if (await focusExisting(worktreeId, label)) {
      if (artifactId) await requestNavigate(label, artifactId);
      return;
    }

    const baseUrl = window.location.origin + window.location.pathname;
    const url =
      `${baseUrl}?mode=artifact&scope=worktree&worktreeId=${encodeURIComponent(worktreeId)}` +
      (artifactId ? `&artifactId=${encodeURIComponent(artifactId)}` : "");

    // タイトルはビューア側が名前を解決してから setTitle で差し替える
    const win = new WebviewWindow(label, {
      url,
      title: "Artifacts",
      width: 900,
      height: 700,
      resizable: true,
      dragDropEnabled: false,
      transparent: true,
    });

    win.once("tauri://error", (e) => {
      console.error(`アーティファクトウィンドウ作成失敗: ${label}`, e);
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
    artifactId?: string,
  ): Promise<void> {
    const key = `repo:${repositoryId}`;
    const label = `artifact-repo-${encodeRepositoryLabel(repositoryId)}`;
    if (await focusExisting(key, label)) {
      if (artifactId) await requestNavigate(label, artifactId);
      return;
    }

    const baseUrl = window.location.origin + window.location.pathname;
    const url =
      `${baseUrl}?mode=artifact&scope=repository&repositoryId=${encodeURIComponent(repositoryId)}` +
      (artifactId ? `&artifactId=${encodeURIComponent(artifactId)}` : "");

    const win = new WebviewWindow(label, {
      url,
      title: "Artifacts",
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
