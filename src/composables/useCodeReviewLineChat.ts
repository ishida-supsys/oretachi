import { ref, onUnmounted } from "vue";
import type { Ref } from "vue";
import { listen, emit } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import type * as Monaco from "monaco-editor";
import type { Worktree } from "../types/worktree";

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

  function updateButtonPosition(editor: Monaco.editor.IStandaloneCodeEditor, sel: Monaco.Selection) {
    const pos = editor.getScrolledVisiblePosition({ lineNumber: sel.endLineNumber, column: sel.endColumn });
    buttonPos.value = pos ? { top: pos.top, left: pos.left, height: pos.height } : null;
  }

  function handleMount(editor: Monaco.editor.IStandaloneCodeEditor) {
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

export function useCodeReviewChat(worktreeId: string) {
  async function handleChatWithAgent(payload: ChatPayload) {
    await emit("codereview-chat-with-agent", { worktreeId, ...payload });
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
}) {
  async function setup() {
    await listen<{ worktreeId: string } & ChatPayload>(
      "codereview-chat-with-agent",
      async (event) => {
        const { worktreeId: wid, filePath, startLine, endLine } = event.payload;
        const wt = deps.worktrees.value.find((w) => w.id === wid);
        if (!wt || wt.terminals.length === 0) return;

        const terminal =
          wt.terminals.find((t) => deps.terminalAgentStatus.get(t.id)) ?? wt.terminals[0];

        const text =
          startLine === endLine
            ? `${filePath}#L${startLine}\n`
            : `${filePath}#L${startLine}-L${endLine}\n`;

        if (deps.isDetached(wid)) {
          const sid = deps.getDetachedSessionId(terminal.id);
          if (sid === null) return;
          await invoke("pty_write", { sessionId: sid, data: Array.from(new TextEncoder().encode(text)) });
        } else {
          const termRef = deps.terminalRefs.get(terminal.id);
          if (!termRef) return;
          await termRef.write(text);
        }
      },
    );
  }

  return { setup };
}
