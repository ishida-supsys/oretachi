<script setup lang="ts">
import { ref, reactive, onMounted, onUnmounted, nextTick, watch, computed } from "vue";
import { emitTo, listen } from "@tauri-apps/api/event";
import { useEventListeners } from "./composables/useEventListeners";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import TerminalView from "./components/TerminalView.vue";
import FrameContainer from "./components/FrameContainer.vue";
import WorktreeHeader from "./components/WorktreeHeader.vue";
import { useWorktreeFrame } from "./composables/useWorktreeFrame";
import { useSettings } from "./composables/useSettings";
import { useHotkeyListener, useAltCharKeyListener } from "./composables/useHotkeys";
import { renderToDataUrl } from "./composables/useTerminalThumbnail";
import { isDirty, clearDirty } from "./composables/usePtyDispatcher";
import { useWindowFocus } from "./composables/useWindowFocus";
import { runApprovalLoop } from "./utils/autoApproval";
import type { TerminalForApproval } from "./utils/autoApproval";
import { useIdeSelect } from "./composables/useIdeSelect";
import { useArtifactWindow } from "./composables/useArtifactWindow";
import { invoke } from "@tauri-apps/api/core";
import { debug } from "@tauri-apps/plugin-log";
import IdeSelectDialog from "./components/IdeSelectDialog.vue";
import AutoApprovalPromptDialog from "./components/AutoApprovalPromptDialog.vue";
import type { SubTerminalEntry, WebSessionInfo } from "./types/terminal";
import type { FrameNode } from "./types/frame";
import { useI18n } from "vue-i18n";

const { t } = useI18n();

// クエリパラメータ
const params = new URLSearchParams(window.location.search);
const worktreeId = params.get("worktreeId") ?? "";
const worktreeName = params.get("worktreeName") ?? "";
const worktreePath = params.get("worktreePath") ?? "";
const branchName = params.get("branchName") ?? "";

// ターミナルエントリ（Map）
const terminalEntries = reactive(new Map<number, SubTerminalEntry>());
const initialized = ref(false);

// terminalId → 直近コマンドの終了コード
const terminalExitCodes = reactive(new Map<number, number>());

// terminalId → AIエージェント稼働中フラグ
const terminalAgentStatus = reactive(new Map<number, boolean>());

// terminalId → Webセッション情報
const terminalWebSessions = reactive(new Map<number, WebSessionInfo>());

// 自動承認フラグ
const autoApproval = ref(false);

// 自動承認 追加プロンプト
const additionalPrompt = ref("");

// 自動承認ダイアログ状態
const showAutoApprovalPromptDialog = ref(false);
const lastJudgedCommand = ref("");

// IDE 選択
const { showIdeDialog, detectedIdes, openInIde, onIdeSelected } = useIdeSelect();

// アーティファクト
const { openArtifactViewer } = useArtifactWindow();

async function requestOpenArtifacts() {
  await openArtifactViewer(worktreeId, worktreeName);
}

async function onSaveAutoApprovalPrompt(wid: string, prompt: string) {
  additionalPrompt.value = prompt.trim();
  showAutoApprovalPromptDialog.value = false;
  await emitTo("main", "sub-save-auto-approval-prompt", { worktreeId: wid, prompt: prompt.trim() });
}

// AI判定進行中フラグ
const aiJudging = ref(false);

// ウィンドウのフォーカス状態
const { isWindowFocused } = useWindowFocus();

// フォーカス状態変化をメインウィンドウに同期
watch(isWindowFocused, (focused) => {
  emitTo("main", "sub-window-focus-changed", { worktreeId, focused });
});

// TerminalView ref 管理
const terminalRefs = reactive(new Map<number, InstanceType<typeof TerminalView>>());

// フレームレイアウト（useWorktreeFrameで共通化）
const {
  root,
  initLayout,
  addTerminalToLeaf,
  lastFocusedLeafId,
  setTerminalRef,
  mountTerminalsToHosts,
  getAllLeafs,
  getLeafsWithTerminals,
  switchTerminal,
  switchNextTerminal,
  switchPrevTerminal,
  closeActiveTerminal,
  closeTerminal,
  handleTerminalExit,
  onSplitRequest,
  onTabDrop,
  onTabEdgeDrop,
  onTabReorder,
} = useWorktreeFrame({
  terminalEntries,
  terminalRefs,
  onTerminalClosed: async (terminalId) => {
    terminalExitCodes.delete(terminalId);
    terminalAgentStatus.delete(terminalId);
    terminalWebSessions.delete(terminalId);
    await emitTo("main", "sub-remove-terminal", { worktreeId, terminalId });
  },
});

// ────────────────────────────────────────────────
// イベントハンドラ
// ────────────────────────────────────────────────

async function requestAddTerminal(leafId?: string) {
  if (leafId) lastFocusedLeafId.value = leafId;
  await emitTo("main", "sub-add-terminal-request", { worktreeId });
}

async function requestOpenInIde() {
  await openInIde(worktreePath, { worktreeId, worktreeName, origin: "sub" });
}

function onTerminalTitleChange(terminalId: number, title: string) {
  const entry = terminalEntries.get(terminalId);
  if (entry) entry.title = title;
  emitTo("main", "sub-title-update", { worktreeId, terminalId, title });
}

function onTerminalFocus(terminalId: number) {
  emitTo("main", "sub-clear-notification", { worktreeId });
  const leaf = getAllLeafs().find((l) => l.terminalIds.includes(terminalId));
  if (leaf) {
    lastFocusedLeafId.value = leaf.id;
  }
}

function getFirstLeafId(): string {
  function find(node: import("./types/frame").FrameNode): string {
    if (node.type === "leaf") return node.id;
    return find(node.children[0]);
  }
  return find(root.value);
}

// ────────────────────────────────────────────────
// イベントリスナー
// ────────────────────────────────────────────────

const { settings, loadSettings } = useSettings();

// このサブウィンドウのホットキー文字
const hotkeyChar = computed(() =>
  settings.value.worktrees.find((w) => w.id === worktreeId)?.hotkeyChar
);

// サブウィンドウのホットキーリスナー
useHotkeyListener(() => {
  const hk = settings.value.hotkeys;
  if (!hk || !initialized.value) return [];

  return [
    {
      binding: hk.homeTab,
      handler: () => {
        WebviewWindow.getByLabel("main").then(async (w) => {
          if (w) {
            await w.setFocus();
            await emitTo("main", "go-home");
          }
        });
      },
    },
    { binding: hk.terminalNext, handler: switchNextTerminal },
    { binding: hk.terminalPrev, handler: switchPrevTerminal },
    { binding: hk.terminalAdd, handler: () => requestAddTerminal(lastFocusedLeafId.value || undefined) },
    { binding: hk.terminalClose, handler: closeActiveTerminal },
  ];
});

// Alt+[char] を受けてメインに委譲（自分自身の hotkeyChar は無視）
useAltCharKeyListener((char, event) => {
  if (char === hotkeyChar.value?.toLowerCase()) return;
  event.preventDefault();
  event.stopPropagation();
  emitTo("main", "sub-alt-char-focus", { char });
});

const { collect } = useEventListeners();
let thumbnailInterval: ReturnType<typeof setInterval> | null = null;
let closingByMain = false;

onMounted(async () => {
  await loadSettings();

  collect(await listen("settings-changed", async () => {
    await loadSettings();
  }));

  // AIエージェントインジケーター: sessionId → terminalId に変換して terminalAgentStatus を更新
  collect(await listen<{ sessions: Record<number, boolean> }>("pty-ai-agent-changed", (event) => {
    const sessionToTerminal = new Map<number, number>();
    for (const [tid, entry] of terminalEntries) {
      if (entry.sessionId) sessionToTerminal.set(entry.sessionId, tid);
    }
    for (const [sessionIdStr, isAgent] of Object.entries(event.payload.sessions)) {
      const sid = Number(sessionIdStr);
      const tid = sessionToTerminal.get(sid);
      if (tid != null) {
        if (isAgent) {
          terminalAgentStatus.set(tid, true);
        } else {
          terminalAgentStatus.delete(tid);
        }
      }
    }
  }));

  // メインウィンドウからのWebセッション情報を受信
  collect(await listen<{ terminalId: number; info: WebSessionInfo }>("sub-web-session", (event) => {
    terminalWebSessions.set(event.payload.terminalId, event.payload.info);
  }));

  const appWindow = getCurrentWindow();

  // 初期ターミナルデータを受信
  collect(await appWindow.listen<{
    worktreeId: string;
    terminals: SubTerminalEntry[];
    autoApproval?: boolean;
    autoApprovalPrompt?: string;
    layout?: FrameNode;
    webSessions?: Record<number, WebSessionInfo>;
  }>(
    "sub-init",
    async (event) => {
      if (event.payload.worktreeId !== worktreeId) return;

      autoApproval.value = event.payload.autoApproval ?? false;
      additionalPrompt.value = event.payload.autoApprovalPrompt ?? "";

      for (const t of event.payload.terminals) {
        terminalEntries.set(t.id, { ...t });
        if (t.isAiAgent) {
          terminalAgentStatus.set(t.id, true);
        }
      }

      if (event.payload.webSessions) {
        for (const [idStr, info] of Object.entries(event.payload.webSessions)) {
          terminalWebSessions.set(Number(idStr), info);
        }
      }

      const ids = event.payload.terminals.map((t) => t.id);

      // レイアウト復元: layout が渡された場合はそのまま設定
      if (event.payload.layout) {
        root.value = event.payload.layout;
      } else {
        initLayout(ids);
      }

      // 最初のリーフを lastFocusedLeafId に設定
      const leafs = getAllLeafs();
      if (leafs.length > 0) {
        lastFocusedLeafId.value = leafs[0].id;
      }

      initialized.value = true;

      // terminal-host が DOM に出るまで待ってから移動
      await nextTick();
      mountTerminalsToHosts();

      const firstLeaf = leafs[0];
      if (firstLeaf?.activeTerminalId !== null && firstLeaf?.activeTerminalId !== undefined) {
        const term = terminalRefs.get(firstLeaf.activeTerminalId);
        if (term) {
          await term.handleTabActivated();
          term.focus();
        }
      }
    }
  ));

  // ターミナル追加レスポンス
  collect(await appWindow.listen<{ terminalId: number; sessionId: number; title: string }>(
    "sub-add-terminal-response",
    async (event) => {
      const { terminalId, sessionId, title } = event.payload;
      terminalEntries.set(terminalId, { id: terminalId, title, sessionId, snapshot: "" });

      // lastFocusedLeafId のリーフに追加（なければ root リーフに）
      const targetLeafId = lastFocusedLeafId.value || getFirstLeafId();
      addTerminalToLeaf(targetLeafId, terminalId);
      lastFocusedLeafId.value = targetLeafId;

      // terminal-host が DOM に出るまで待ってから移動
      await nextTick();
      mountTerminalsToHosts();

      const term = terminalRefs.get(terminalId);
      if (term) {
        await term.handleTabActivated();
        term.focus();
      }
    }
  ));

  // メインウィンドウからのフォーカスリクエスト
  collect(await appWindow.listen<{ terminalId: number }>(
    "sub-focus-terminal",
    async (event) => {
      const { terminalId } = event.payload;
      const leaf = getLeafsWithTerminals().find((l) => l.terminalIds.includes(terminalId));
      if (leaf) {
        await switchTerminal(leaf.id, terminalId);
      }
    }
  ));

  // メインウィンドウが「メインに戻す」を選択
  collect(await appWindow.listen<{ worktreeId: string }>(
    "sub-closing-by-main",
    async (event) => {
      if (event.payload.worktreeId === worktreeId) {
        closingByMain = true;
        // 全ターミナルの PTY を detach (メイン側で引き継ぐため kill しない)
        for (const [, entry] of terminalEntries) {
          const termRef = terminalRefs.get(entry.id);
          if (termRef?.isRunning) {
            termRef.detach();
          }
        }
        // kill 完了をメインに通知（destroy 前に送信）
        await emitTo("main", "sub-window-closed-ack", { worktreeId });
        await appWindow.destroy();
      }
    }
  ));

  // X ボタンでクローズ
  appWindow.onCloseRequested(async (event) => {
    event.preventDefault();
    if (!closingByMain) {
      for (const [, entry] of terminalEntries) {
        const termRef = terminalRefs.get(entry.id);
        if (termRef?.isRunning) {
          await termRef.kill();
        }
      }
      await emitTo("main", "sub-window-closing", { worktreeId });
    }
    await appWindow.destroy();
  });

  // サムネイル送信ループ（pty出力があった場合のみ送信）
  const lastThumbnailUrls = new Map<number, string>();
  thumbnailInterval = setInterval(() => {
    for (const [id] of terminalEntries) {
      const ref = terminalRefs.get(id);
      const sid = ref?.sessionId;
      if (sid == null || !isDirty(sid)) continue;
      clearDirty(sid);
      const terminal = ref?.getTerminal();
      if (terminal) {
        const url = renderToDataUrl(terminal);
        if (url && url !== lastThumbnailUrls.get(id)) {
          lastThumbnailUrls.set(id, url);
          emitTo("main", "sub-thumbnail-update", { terminalId: id, imageUrl: url });
        }
      }
    }
  }, 1000);

  // メインウィンドウからのレイアウト情報取得要求
  collect(await appWindow.listen("sub-get-layout", async () => {
    const layout = JSON.parse(JSON.stringify(root.value));
    const terminals = Array.from(terminalEntries.values()).map((entry) => {
      const termRef = terminalRefs.get(entry.id);
      const snapshot = termRef?.serializeBuffer(300) ?? entry.snapshot;
      const termObj = termRef?.getTerminal();
      return {
        id: entry.id,
        title: entry.title,
        sessionId: entry.sessionId,
        snapshot,
        isAiAgent: entry.isAiAgent ?? false,
        rows: termObj?.rows ?? 24,
        cols: termObj?.cols ?? 80,
      };
    });
    const physicalSize = await appWindow.innerSize();
    const scaleFactor = await appWindow.scaleFactor();
    const windowSize = {
      width: Math.round(physicalSize.width / scaleFactor),
      height: Math.round(physicalSize.height / scaleFactor),
    };
    await emitTo("main", "sub-layout-response", { worktreeId, layout, terminals, windowSize });
  }));

  // 自動承認フラグ更新
  collect(await appWindow.listen<{ autoApproval: boolean }>(
    "sub-set-auto-approval",
    (event) => {
      autoApproval.value = event.payload.autoApproval;
    }
  ));

  // 自動承認 追加プロンプト更新
  collect(await appWindow.listen<{ prompt: string }>(
    "sub-set-auto-approval-prompt",
    (event) => {
      additionalPrompt.value = event.payload.prompt;
    }
  ));

  // 自動承認チェック（notify-worktree トリガー）
  collect(await appWindow.listen<{ additionalPrompt?: string }>("sub-try-auto-approve", async (event) => {
    if (event.payload.additionalPrompt !== undefined) {
      additionalPrompt.value = event.payload.additionalPrompt;
    }
    await debug(`[AutoApproval] sub-try-auto-approve received autoApproval=${autoApproval.value}`);
    if (!autoApproval.value) {
      await emitTo("main", "sub-auto-approve-result", { worktreeId, approved: false });
      return;
    }

    // 重複防止: 既にAI判定が進行中ならスキップ
    if (aiJudging.value) {
      await debug(`[AutoApproval] already in progress for sub-window ${worktreeId}, skipping`);
      return;
    }

    await debug(`[AutoApproval] terminalEntries.size=${terminalEntries.size}`);
    aiJudging.value = true;
    let loopResult: { approved: boolean; lastCommand: string | undefined };
    try {
      const terminalForApproval: TerminalForApproval[] = Array.from(terminalEntries.keys()).flatMap((tid) => {
        const ref = terminalRefs.get(tid);
        if (!ref) return [];
        return [{ id: tid, getTerminal: () => ref.getTerminal(), write: (d: string) => ref.write(d) }];
      });
      loopResult = await runApprovalLoop(terminalForApproval, worktreeId, worktreePath, additionalPrompt.value);
    } finally {
      aiJudging.value = false;
    }
    await debug(`[AutoApproval] sub result: approved=${loopResult.approved} command=${loopResult.lastCommand ?? "none"}`);
    if (loopResult.lastCommand) lastJudgedCommand.value = loopResult.lastCommand;
    await emitTo("main", "sub-auto-approve-result", { worktreeId, approved: loopResult.approved, command: loopResult.lastCommand });
  }));

  // AI判定キャンセル
  collect(await appWindow.listen("sub-cancel-auto-approve", async () => {
    await debug(`[AutoApproval] sub-cancel-auto-approve received`);
    await invoke("cancel_approval", { worktreeId });
    aiJudging.value = false;
  }));

  // セッション保存リクエスト
  collect(await appWindow.listen("sub-session-save-request", async () => {
    const terminals = Array.from(terminalEntries.values()).map((entry) => {
      const termRef = terminalRefs.get(entry.id);
      return { title: entry.title, buffer: termRef?.serializeBuffer() ?? "" };
    }).filter((t) => t.buffer !== "");
    await emitTo("main", "sub-session-save-response", { worktreeId, terminals });
  }));

  // メインに準備完了を通知
  await emitTo("main", "sub-ready", { worktreeId });
});

onUnmounted(() => {
  if (thumbnailInterval) clearInterval(thumbnailInterval);
});

async function onCancelAiJudging() {
  await invoke("cancel_approval", { worktreeId });
  aiJudging.value = false;
}
</script>

<template>
  <div
    class="h-screen flex flex-col text-[#cdd6f4] select-none"
    style="background-color: var(--bg-base)"
    :class="[{ 'gaming-border': settings.appearance?.enableGamingBorder }, settings.appearance?.enableGamingBorder ? `gaming-theme-${settings.appearance?.gamingBorderTheme ?? 'gaming'}` : '']"
  >
    <!-- 初期化中 -->
    <div v-if="!initialized" class="flex items-center justify-center h-full text-[#6c7086] text-sm">
      {{ t('connecting') }}
    </div>

    <template v-else>
      <!-- ヘッダー -->
      <WorktreeHeader
        :worktree-name="worktreeName"
        :branch-name="branchName"
        :hotkey-char="hotkeyChar"
        :auto-approval="autoApproval"
        :ai-judging="aiJudging"
        :is-window-focused="isWindowFocused"
        :show-window-controls="true"
        @open-in-ide="requestOpenInIde"
        @open-artifacts="requestOpenArtifacts"
        @cancel-ai-judging="onCancelAiJudging"
        @click-auto-approval="showAutoApprovalPromptDialog = true"
      />

      <!-- フレームレイアウト -->
      <div class="flex-1 min-h-0 overflow-hidden">
        <FrameContainer
          :node="root"
          :terminal-entries="terminalEntries"
          :terminal-exit-codes="terminalExitCodes"
          :terminal-agent-status="terminalAgentStatus"
          :terminal-web-sessions="terminalWebSessions"
          @switch-terminal="switchTerminal"
          @close-terminal="closeTerminal"
          @title-change="onTerminalTitleChange"
          @split-request="onSplitRequest"
          @tab-drop="onTabDrop"
          @tab-edge-drop="onTabEdgeDrop"
          @tab-reorder="onTabReorder"
          @request-add-terminal="requestAddTerminal"
          @resize-end="() => {}"
        />
      </div>
    </template>

    <!-- IDE 選択ダイアログ -->
    <IdeSelectDialog
      v-if="showIdeDialog"
      :ides="detectedIdes"
      @select="onIdeSelected"
      @cancel="showIdeDialog = false"
    />

    <!-- 自動承認 追加プロンプト編集ダイアログ -->
    <AutoApprovalPromptDialog
      v-if="showAutoApprovalPromptDialog"
      :worktree-id="worktreeId"
      :worktree-name="worktreeName"
      :current-prompt="additionalPrompt"
      :last-command="lastJudgedCommand"
      @save="onSaveAutoApprovalPrompt"
      @cancel="showAutoApprovalPromptDialog = false"
    />

    <!-- TerminalView のマウント先。手動 DOM reparenting で terminal-host に移動する -->
    <div data-offscreen style="position:fixed; left:-10000px; top:-10000px; width:1000px; height:1000px; overflow:hidden; pointer-events:none">
      <template v-for="[tid, entry] in terminalEntries" :key="tid">
        <TerminalView
          :ref="(el) => setTerminalRef(tid, el)"
          :auto-start="false"
          :initial-session-id="entry.sessionId"
          :initial-snapshot="entry.snapshot"
          @exit="handleTerminalExit(tid)"
          @title-change="onTerminalTitleChange(tid, $event)"
          @focus="() => onTerminalFocus(tid)"
          @exit-code-change="(code: number) => terminalExitCodes.set(tid, code)"
        />
      </template>
    </div>
  </div>
</template>

<i18n lang="json">
{
  "en": {
    "connecting": "Connecting...",
    "ideNotInstalled": "None of Cursor, VS Code, Antigravity are installed.",
    "ideNotInstalledTitle": "IDE not found",
    "ideLaunchFailed": "Failed to launch IDE: {error}"
  },
  "ja": {
    "connecting": "接続中...",
    "ideNotInstalled": "Cursor、VS Code、Antigravity のいずれもインストールされていません。",
    "ideNotInstalledTitle": "IDE が見つかりません",
    "ideLaunchFailed": "IDE の起動に失敗しました: {error}"
  }
}
</i18n>
