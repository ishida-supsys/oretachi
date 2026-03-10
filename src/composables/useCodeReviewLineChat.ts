import { ref, onUnmounted } from "vue";
import type { Ref } from "vue";
import { listen, emit, emitTo } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import type * as Monaco from "monaco-editor";
import type { Worktree } from "../types/worktree";
import type { WorktreeFrameBundle } from "./useWorktreeFrameBundles";

export type CodeReviewOrigin = "main" | "sub" | "tray";

// ─── 共通ペイロード型 ──────────────────────────────────────────────────────

export interface ChatPayload {
  filePath: string;
  startLine: number;
  endLine: number;
}

// ─── useEditorLineSelection ────────────────────────────────────────────────
// MonacoFileViewer.vue: エディタ行選択検知とチャットボタン位置管理

export function useEditorLineSelection(
  filePath: () => string | undefined,
  onChat: (payload: ChatPayload) => void,
) {
  const selectionInfo = ref<{ startLine: number; endLine: number } | null>(null);
  const buttonPos = ref<{ top: number; left: number; height: number } | null>(null);
  let disposeListeners: (() => void) | null = null;

  function updateButtonPosition(editor: Monaco.editor.ICodeEditor, sel: Monaco.Selection) {
    const pos = editor.getScrolledVisiblePosition({ lineNumber: sel.endLineNumber, column: sel.endColumn });
    buttonPos.value = pos ? { top: pos.top, left: pos.left, height: pos.height } : null;
  }

  function handleMount(editor: Monaco.editor.ICodeEditor) {
    const selListener = editor.onDidChangeCursorSelection(() => {
      const sel = editor.getSelection();
      if (!sel || sel.isEmpty()) {
        selectionInfo.value = null;
        buttonPos.value = null;
        return;
      }
      selectionInfo.value = { startLine: sel.startLineNumber, endLine: sel.endLineNumber };
      updateButtonPosition(editor, sel);
    });

    const scrollListener = editor.onDidScrollChange(() => {
      if (!selectionInfo.value) return;
      const sel = editor.getSelection();
      if (!sel || sel.isEmpty()) return;
      updateButtonPosition(editor, sel);
    });

    disposeListeners = () => {
      selListener.dispose();
      scrollListener.dispose();
    };
  }

  function handleChatClick() {
    const fp = filePath();
    if (!selectionInfo.value || !fp) return;
    onChat({ filePath: fp, startLine: selectionInfo.value.startLine, endLine: selectionInfo.value.endLine });
  }

  onUnmounted(() => disposeListeners?.());

  return { selectionInfo, buttonPos, handleMount, handleChatClick };
}

// ─── useCodeReviewChat ─────────────────────────────────────────────────────
// CodeReviewApp.vue: チャットイベントをメインウィンドウに中継

export function useCodeReviewChat(worktreeId: string, origin: CodeReviewOrigin) {
  async function handleChatWithAgent(payload: ChatPayload) {
    await emit("codereview-chat-with-agent", { worktreeId, origin, ...payload });
  }
  return { handleChatWithAgent };
}

// ─── useCodeReviewChatListener ─────────────────────────────────────────────
// App.vue: codereview-chat-with-agent を受信しターミナルへ書き込む

interface TerminalRef {
  write(data: string): Promise<void>;
}

export function useCodeReviewChatListener(deps: {
  worktrees: Ref<Worktree[]>;
  terminalAgentStatus: Map<number, boolean>;
  isDetached: (worktreeId: string) => boolean;
  getDetachedSessionId: (terminalId: number) => number | null;
  terminalRefs: Map<number, TerminalRef>;
  worktreeFrameBundles: Map<string, WorktreeFrameBundle>;
  activeWorktreeId: Ref<string | null>;
  switchToWorktree: (worktreeId: string) => Promise<void>;
  focusSubWindow: (worktreeId: string) => Promise<void>;
  focusMainWindow: () => Promise<void>;
  isTrayShowingWorktree: (worktreeId: string) => boolean;
  focusTrayPopup: () => Promise<void>;
}) {
  async function setup() {
    await listen<{ worktreeId: string; origin: CodeReviewOrigin } & ChatPayload>(
      "codereview-chat-with-agent",
      async (event) => {
        const { worktreeId: wid, origin, filePath, startLine, endLine } = event.payload;
        const wt = deps.worktrees.value.find((w) => w.id === wid);
        if (!wt || wt.terminals.length === 0) return;

        const terminal =
          wt.terminals.find((t) => deps.terminalAgentStatus.get(t.id)) ?? wt.terminals[0];

        const text =
          startLine === endLine
            ? `${filePath}#L${startLine}`
            : `${filePath}#L${startLine}-L${endLine}`;

        // テキスト書き込み
        if (deps.isDetached(wid)) {
          const sid = deps.getDetachedSessionId(terminal.id);
          if (sid === null) return;
          await invoke("pty_write", { sessionId: sid, data: Array.from(new TextEncoder().encode(text)) });
        } else {
          const termRef = deps.terminalRefs.get(terminal.id);
          if (!termRef) return;
          await termRef.write(text);
        }

        // 起動元に応じたフォーカス処理
        if (origin === "tray" && deps.isTrayShowingWorktree(wid)) {
          // トレイポップアップが同じワークツリーを表示中 → トレイにフォーカス
          await deps.focusTrayPopup();
        } else if (deps.isDetached(wid)) {
          // サブウィンドウ → サブウィンドウにフォーカス + ターミナルタブ切替
          await deps.focusSubWindow(wid);
          await emitTo(`sub-${wid}`, "sub-focus-terminal", { terminalId: terminal.id });
        } else {
          // メインウィンドウ → ワークツリー切替 + ターミナルタブ切替 + メインウィンドウにフォーカス
          if (deps.activeWorktreeId.value !== wid) {
            await deps.switchToWorktree(wid);
          }
          const bundle = deps.worktreeFrameBundles.get(wid);
          if (bundle) {
            const leaf = bundle.frame.getAllLeafs().find((l) => l.terminalIds.includes(terminal.id));
            if (leaf) {
              await bundle.frame.switchTerminal(leaf.id, terminal.id);
            }
          }
          await deps.focusMainWindow();
        }
      },
    );
  }

  return { setup };
}
