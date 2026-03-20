import { reactive } from "vue";
import { listen } from "@tauri-apps/api/event";
import type { Ref } from "vue";
import type { Worktree } from "../types/worktree";
import type { SubTerminalEntry, WebSessionInfo } from "../types/terminal";
import type { SubLayoutResponse } from "./useSubWindows";

interface UseSubWindowEventsDeps {
  worktrees: Ref<Worktree[]>;
  removeTerminal: (worktreeId: string, terminalId: number) => void;
  unregisterSubWindow: (worktreeId: string) => void;
  terminalWorktreeMap: Map<number, string>;
  thumbnailUrls: Map<number, string>;
  terminalAgentStatus: Map<number, boolean>;
  terminalWebSessions: Map<number, WebSessionInfo>;
  updateTerminalTitle: (worktreeId: string, terminalId: number, title: string) => void;
  clearNotification: (worktreeId: string) => void;
  requestSubWindowLayout: (worktreeId: string) => Promise<SubLayoutResponse | null>;
}

export function useSubWindowEvents(deps: UseSubWindowEventsDeps) {
  const subWindowFocusMap = reactive(new Map<string, boolean>());

  async function getSubWindowLayout(worktreeId: string) {
    const payload = await deps.requestSubWindowLayout(worktreeId);
    return {
      layout: payload?.layout ?? null,
      terminals: (payload?.terminals ?? []) as SubTerminalEntry[],
    };
  }

  async function init() {
    // サブウィンドウ close 通知
    await listen<{ worktreeId: string }>("sub-window-closing", async (event) => {
      const { worktreeId } = event.payload;
      subWindowFocusMap.delete(worktreeId);
      deps.unregisterSubWindow(worktreeId);
      const worktree = deps.worktrees.value.find((w) => w.id === worktreeId);
      if (worktree) {
        for (const terminal of [...worktree.terminals]) {
          deps.terminalWorktreeMap.delete(terminal.id);
        }
      }
    });

    // サブウィンドウのフォーカス状態変化
    await listen<{ worktreeId: string; focused: boolean }>("sub-window-focus-changed", (event) => {
      subWindowFocusMap.set(event.payload.worktreeId, event.payload.focused);
    });

    // サブウィンドウでターミナルが削除された通知
    await listen<{ worktreeId: string; terminalId: number }>("sub-remove-terminal", (event) => {
      const { worktreeId, terminalId } = event.payload;
      deps.removeTerminal(worktreeId, terminalId);
      deps.terminalWorktreeMap.delete(terminalId);
      deps.thumbnailUrls.delete(terminalId);
      deps.terminalAgentStatus.delete(terminalId);
      deps.terminalWebSessions.delete(terminalId);
    });

    // サブウィンドウからのサムネイル受信
    await listen<{ terminalId: number; imageUrl: string }>("sub-thumbnail-update", (event) => {
      deps.thumbnailUrls.set(event.payload.terminalId, event.payload.imageUrl);
    });

    // サブウィンドウからのタイトル変更通知
    await listen<{ worktreeId: string; terminalId: number; title: string }>(
      "sub-title-update",
      (event) => {
        const { worktreeId: wid, terminalId, title } = event.payload;
        deps.updateTerminalTitle(wid, terminalId, title);
      },
    );

    // サブウィンドウでのターミナルフォーカス時の通知クリア
    await listen<{ worktreeId: string }>("sub-clear-notification", (event) => {
      deps.clearNotification(event.payload.worktreeId);
    });
  }

  return { subWindowFocusMap, getSubWindowLayout, init };
}
