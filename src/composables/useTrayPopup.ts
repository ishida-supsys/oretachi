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
  windowSize?: { width: number; height: number }; // 論理px (DIP)。CSS px 計測値はプロデューサ側でズーム換算済み
  branchName: string;
  repositoryName: string;
  hotkeyChar?: string;
  autoApproval: boolean;
  aiJudging: boolean;
  autoApprovalPrompt?: string;
  lastJudgedCommand?: string;
  description?: string; // ホーム画面と同じ AI 生成説明。トレイの情報バーに表示
  // アーカイブ操作を許可するか。ホーム/リポジトリ擬似ワークツリーもターミナルを持つため
  // 通知対象になりトレイに載るが、git worktree ではないのでアーカイブしてはいけない
  canArchive: boolean;
}

const TRAY_LABEL = "tray-popup";
/** tauri://created / error が届かない場合の保険。待機側を永久に止めないための上限 */
const CREATE_TIMEOUT_MS = 5000;

let trayWindow: WebviewWindow | null = null;
/**
 * 生成要求を出してから tauri://created / error（または保険のタイムアウト）が来るまで解決しない。
 * `new WebviewWindow` は要求を投げるだけで完了を待たないため、この間に再度呼ばれると
 * setFocus が失敗して「無い」と誤判定し、同じラベルでもう1枚作ってしまう (#432)。
 */
let openingPromise: Promise<void> | null = null;
/**
 * 生成要求は出したが created / error をまだ受け取っていない窓。
 * 保険のタイムアウトは待機を解くだけで、この窓を「無い」と見なす根拠にはしない。
 * 生成が CREATE_TIMEOUT_MS を超えて遅いとき、未登録の窓への setFocus 失敗 → getByLabel が null →
 * 作り直し、と進んで #432 と同じ二重生成になるため。
 */
let pendingWin: WebviewWindow | null = null;

export function useTrayPopup() {
  async function openTrayPopup(worktrees: TrayWorktreeData[]): Promise<void> {
    // await を挟むたびに状態が変わり得るので、判定はループで毎回やり直す
    for (;;) {
      if (openingPromise) {
        await openingPromise;
        continue;
      }
      const win = trayWindow;
      if (!win) break;

      try {
        await win.setFocus();
        return;
      } catch {
        // フォーカス失敗 = 消えたとは限らない。下で実在を確かめる
      }
      if (trayWindow !== win) continue; // await 中に close / 差し替えがあった

      const existing = await WebviewWindow.getByLabel(TRAY_LABEL).catch(() => null);
      if (trayWindow !== win) continue;
      if (existing) {
        // 実在するのに作り直すと同ラベルが2枚になる。フォーカスだけ試して終える
        trayWindow = existing;
        await existing.setFocus().catch(() => {});
        return;
      }
      // 生成要求がまだ返っていない窓は、実在確認が取れなくても作り直さない
      if (pendingWin === win) return;
      trayWindow = null; // 本当に無い。次の周回で生成へ進む
    }

    // ここから生成。new WebviewWindow と openingPromise の代入の間に await を挟まない
    const baseUrl = window.location.origin + window.location.pathname;
    const url = `${baseUrl}?mode=tray`;

    const win = new WebviewWindow(TRAY_LABEL, {
      url,
      title: "oretachi - 通知",
      width: 900,
      height: 600,
      resizable: true,
      decorations: false,
      dragDropEnabled: false,
      transparent: true,
    });
    trayWindow = win;
    pendingWin = win;

    // ペンディングデータを保持（tray-ready 受信後に main が送信する）
    // worktrees データは App.vue 側が tray-ready → tray-init で送信する
    // ここでは参照のために pendingWorktrees に保持しておく
    pendingWorktrees = worktrees;

    const opening = new Promise<void>((resolve) => {
      const timer = setTimeout(resolve, CREATE_TIMEOUT_MS);
      const settle = () => {
        if (pendingWin === win) pendingWin = null;
      };
      const done = () => {
        clearTimeout(timer);
        resolve();
      };
      win
        .once("tauri://created", () => {
          settle();
          done();
        })
        .catch(() => {});
      win
        .once("tauri://error", (e) => {
          console.error("トレイポップアップ作成失敗:", e);
          settle();
          // close 後に作り直された別個体の状態を壊さないよう、自分が現役のときだけ戻す
          if (trayWindow === win) {
            trayWindow = null;
            pendingWorktrees = null;
          }
          done();
        })
        .catch(() => {});
    });
    openingPromise = opening;
    void opening.then(() => {
      if (openingPromise === opening) openingPromise = null;
    });
  }

  async function closeTrayPopup(skipDestroy = false): Promise<void> {
    const win = trayWindow;
    trayWindow = null;
    openingPromise = null;
    pendingWin = null;
    pendingWorktrees = null;
    currentTrayWorktreeId = null;
    if (win && !skipDestroy) {
      await win.destroy().catch(() => {});
    }
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

  /** トレイポップアップが開いているか。開いている間だけ届く更新イベントの送信判定に使う */
  function isTrayPopupOpen(): boolean {
    return trayWindow !== null;
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
    isTrayPopupOpen,
    isTrayShowingWorktree,
    focusTrayWindow,
  };
}

// モジュールスコープで pendingWorktrees を保持
let pendingWorktrees: TrayWorktreeData[] | null = null;
// トレイポップアップが現在表示しているワークツリーID
let currentTrayWorktreeId: string | null = null;
