<script setup lang="ts">
import { ref, reactive, onMounted, onUnmounted, nextTick, watch, computed } from "vue";
import { emitTo, listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import TerminalView from "./components/TerminalView.vue";
import FrameContainer from "./components/FrameContainer.vue";
import { useFrameLayout } from "./composables/useFrameLayout";
import { useSettings } from "./composables/useSettings";
import { useHotkeyListener } from "./composables/useHotkeys";
import { renderToDataUrl } from "./composables/useTerminalThumbnail";
import { useWindowFocus } from "./composables/useWindowFocus";
import { getRecentLines, analyzeForApproval, hasApprovalPrompt } from "./utils/autoApproval";
import { useTerminalReparenting } from "./composables/useTerminalReparenting";
import { useIdeSelect } from "./composables/useIdeSelect";
import { invoke } from "@tauri-apps/api/core";
import { debug } from "@tauri-apps/plugin-log";
import IdeSelectDialog from "./components/IdeSelectDialog.vue";
import type { SubTerminalEntry } from "./types/terminal";
import { useI18n } from "vue-i18n";

const { t } = useI18n();

// クエリパラメータ
const params = new URLSearchParams(window.location.search);
const worktreeId = params.get("worktreeId") ?? "";
const worktreeName = params.get("worktreeName") ?? "";
const worktreePath = params.get("worktreePath") ?? "";

// ターミナルエントリ（Map）
const terminalEntries = reactive(new Map<number, SubTerminalEntry>());
const initialized = ref(false);

// terminalId → 直近コマンドの終了コード
const terminalExitCodes = reactive(new Map<number, number>());

// 自動承認フラグ
const autoApproval = ref(false);

// IDE 選択
const { showIdeDialog, detectedIdes, openInIde, onIdeSelected } = useIdeSelect();

// AI判定進行中フラグ
const aiJudging = ref(false);

// ウィンドウのフォーカス状態
const { isWindowFocused } = useWindowFocus();

// フォーカス状態変化をメインウィンドウに同期
watch(isWindowFocused, (focused) => {
  emitTo("main", "sub-window-focus-changed", { worktreeId, focused });
});

// フレームレイアウト
const { root, initLayout, addTerminalToLeaf, removeTerminalFromLeaf, moveTerminal, setActiveTerminal, splitLeaf, pruneTree, findLeafByTerminalId } = useFrameLayout();

// 最後にフォーカスされたリーフ（新ターミナル追加先）
const lastFocusedLeafId = ref<string>("");

// TerminalView ref 管理
const terminalRefs = reactive(new Map<number, InstanceType<typeof TerminalView>>());

const { setTerminalRef, returnAllToOffscreen, mountTerminalsToHosts } =
  useTerminalReparenting(terminalEntries, terminalRefs);

// PTY 終了時にリーフを特定して closeTerminal を呼ぶ
function handleTerminalExit(tid: number) {
  const leaf = findLeafByTerminalId(tid);
  if (leaf) {
    closeTerminal(leaf.id, tid);
  }
}

// ────────────────────────────────────────────────
// イベントハンドラ
// ────────────────────────────────────────────────

async function switchTerminal(leafId: string, terminalId: number) {
  setActiveTerminal(leafId, terminalId);
  lastFocusedLeafId.value = leafId;
  await nextTick();
  const term = terminalRefs.get(terminalId);
  if (term) {
    await term.handleTabActivated();
    term.focus();
  }
}

async function closeTerminal(leafId: string, terminalId: number) {
  const entry = terminalEntries.get(terminalId);
  if (!entry) return;

  const term = terminalRefs.get(terminalId);
  if (term?.isRunning) {
    await term.kill();
  }

  // kill 中に pty-exit → 再入で既に削除済みの可能性
  if (!terminalEntries.has(terminalId)) return;

  returnAllToOffscreen();

  terminalEntries.delete(terminalId);
  terminalExitCodes.delete(terminalId);
  removeTerminalFromLeaf(leafId, terminalId);
  pruneTree();

  await emitTo("main", "sub-remove-terminal", { worktreeId, terminalId });

  await nextTick();
  mountTerminalsToHosts();

  // 全ターミナルに handleTabActivated
  for (const [tid] of terminalEntries) {
    const t = terminalRefs.get(tid);
    if (t) await t.handleTabActivated();
  }

  // 削除後にアクティブなターミナルにフォーカス
  const leafs = getLeafsWithTerminals();
  if (leafs.length > 0) {
    const firstLeaf = leafs[0];
    if (firstLeaf.activeTerminalId !== null) {
      const activeTerm = terminalRefs.get(firstLeaf.activeTerminalId);
      if (activeTerm) {
        activeTerm.focus();
      }
    }
  }
}

function getLeafsWithTerminals() {
  const leafs: import("./types/frame").FrameLeaf[] = [];
  function collect(node: import("./types/frame").FrameNode) {
    if (node.type === "leaf") {
      if (node.terminalIds.length > 0) leafs.push(node);
    } else {
      node.children.forEach(collect);
    }
  }
  collect(root.value);
  return leafs;
}

async function requestAddTerminal(leafId?: string) {
  if (leafId) lastFocusedLeafId.value = leafId;
  await emitTo("main", "sub-add-terminal-request", { worktreeId });
}

async function requestOpenInIde() {
  await openInIde(worktreePath);
}

function onTerminalTitleChange(terminalId: number, title: string) {
  const entry = terminalEntries.get(terminalId);
  if (entry) entry.title = title;
  emitTo("main", "sub-title-update", { worktreeId, terminalId, title });
}

function onTerminalFocus(terminalId: number) {
  emitTo("main", "sub-clear-notification", { worktreeId });
  const leaf = findLeafByTerminalId(terminalId);
  if (leaf) {
    lastFocusedLeafId.value = leaf.id;
  }
}

async function onSplitRequest(leafId: string, direction: "left" | "right" | "top" | "bottom") {
  returnAllToOffscreen();

  splitLeaf(leafId, direction);
  lastFocusedLeafId.value = leafId;

  await nextTick();
  mountTerminalsToHosts();

  // 全ターミナルに handleTabActivated
  for (const [tid] of terminalEntries) {
    const term = terminalRefs.get(tid);
    if (term) await term.handleTabActivated();
  }
}

function onTabReorder(leafId: string, terminalId: number, insertIndex: number) {
  moveTerminal(terminalId, leafId, leafId, insertIndex);
  // 同一リーフ内の並び替えのため pruneTree / DOM reparent 不要
}

async function onTabDrop(sourceLeafId: string, terminalId: number, targetLeafId: string, insertIndex?: number) {
  if (sourceLeafId === targetLeafId) return;

  returnAllToOffscreen();

  moveTerminal(terminalId, sourceLeafId, targetLeafId, insertIndex);
  pruneTree();
  lastFocusedLeafId.value = targetLeafId;

  await nextTick();
  mountTerminalsToHosts();

  // 全ターミナルに handleTabActivated
  for (const [tid] of terminalEntries) {
    const term = terminalRefs.get(tid);
    if (term) await term.handleTabActivated();
  }
  const movedTerm = terminalRefs.get(terminalId);
  if (movedTerm) movedTerm.focus();
}

async function onTabEdgeDrop(
  sourceLeafId: string,
  terminalId: number,
  targetLeafId: string,
  direction: "left" | "right" | "top" | "bottom"
) {
  returnAllToOffscreen();

  const newLeaf = splitLeaf(targetLeafId, direction);
  moveTerminal(terminalId, sourceLeafId, newLeaf.id);
  pruneTree();
  lastFocusedLeafId.value = newLeaf.id;

  await nextTick();
  mountTerminalsToHosts();

  for (const [tid] of terminalEntries) {
    const term = terminalRefs.get(tid);
    if (term) {
      await term.handleTabActivated();
    }
  }

  const movedTerm = terminalRefs.get(terminalId);
  if (movedTerm) movedTerm.focus();
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
      binding: hk.focusMainWindow,
      handler: () => {
        WebviewWindow.getByLabel("main").then((w) => w?.setFocus());
      },
    },
    {
      binding: hk.terminalNext,
      handler: () => {
        const leafId = lastFocusedLeafId.value;
        if (!leafId) return;
        const leaf = getLeafsWithTerminals().find((l) => l.id === leafId);
        if (!leaf || leaf.terminalIds.length === 0) return;
        const idx = leaf.terminalIds.indexOf(leaf.activeTerminalId ?? -1);
        const nextIdx = idx === -1 ? 0 : (idx + 1) % leaf.terminalIds.length;
        switchTerminal(leafId, leaf.terminalIds[nextIdx]);
      },
    },
    {
      binding: hk.terminalPrev,
      handler: () => {
        const leafId = lastFocusedLeafId.value;
        if (!leafId) return;
        const leaf = getLeafsWithTerminals().find((l) => l.id === leafId);
        if (!leaf || leaf.terminalIds.length === 0) return;
        const idx = leaf.terminalIds.indexOf(leaf.activeTerminalId ?? -1);
        const prevIdx = idx <= 0 ? leaf.terminalIds.length - 1 : idx - 1;
        switchTerminal(leafId, leaf.terminalIds[prevIdx]);
      },
    },
    {
      binding: hk.terminalAdd,
      handler: () => {
        requestAddTerminal(lastFocusedLeafId.value || undefined);
      },
    },
    {
      binding: hk.terminalClose,
      handler: () => {
        const leafId = lastFocusedLeafId.value;
        if (!leafId) return;
        const leaf = getLeafsWithTerminals().find((l) => l.id === leafId);
        if (leaf?.activeTerminalId !== null && leaf?.activeTerminalId !== undefined) {
          closeTerminal(leafId, leaf.activeTerminalId);
        }
      },
    },
  ];
});

// Alt+[char] を受けてメインに委譲
function handleAltCharKey(event: KeyboardEvent) {
  if (event.type !== "keydown") return;
  if (event.isComposing || event.keyCode === 229) return;
  if (!event.altKey || event.ctrlKey || event.shiftKey) return;
  if (event.key.length !== 1) return;

  const char = event.key.toLowerCase();
  // 自分自身のホットキー文字は無視
  if (char === hotkeyChar.value?.toLowerCase()) return;

  event.preventDefault();
  event.stopPropagation();
  emitTo("main", "sub-alt-char-focus", { char });
}

let unlistenInit: UnlistenFn | null = null;
let unlistenAddResponse: UnlistenFn | null = null;
let unlistenFocusTerminal: UnlistenFn | null = null;
let unlistenClosingByMain: UnlistenFn | null = null;
let unlistenGetLayout: UnlistenFn | null = null;
let unlistenSetAutoApproval: UnlistenFn | null = null;
let unlistenTryAutoApprove: UnlistenFn | null = null;
let unlistenCancelAutoApprove: UnlistenFn | null = null;
let unlistenSettingsChanged: UnlistenFn | null = null;
let unlistenSessionSaveRequest: UnlistenFn | null = null;
let thumbnailInterval: ReturnType<typeof setInterval> | null = null;
let closingByMain = false;

onMounted(async () => {
  await loadSettings();
  window.addEventListener("keydown", handleAltCharKey, true);

  unlistenSettingsChanged = await listen("settings-changed", async () => {
    await loadSettings();
  });

  const appWindow = getCurrentWindow();

  // 初期ターミナルデータを受信
  unlistenInit = await appWindow.listen<{ worktreeId: string; terminals: SubTerminalEntry[]; autoApproval?: boolean }>(
    "sub-init",
    async (event) => {
      if (event.payload.worktreeId !== worktreeId) return;

      autoApproval.value = event.payload.autoApproval ?? false;

      for (const t of event.payload.terminals) {
        terminalEntries.set(t.id, { ...t });
      }

      const ids = event.payload.terminals.map((t) => t.id);
      initLayout(ids);

      // 最初のリーフを lastFocusedLeafId に設定
      if (root.value.type === "leaf") {
        lastFocusedLeafId.value = root.value.id;
      }

      initialized.value = true;

      // terminal-host が DOM に出るまで待ってから移動
      await nextTick();
      mountTerminalsToHosts();

      if (root.value.type === "leaf" && root.value.activeTerminalId !== null) {
        const term = terminalRefs.get(root.value.activeTerminalId);
        if (term) {
          await term.handleTabActivated();
          term.focus();
        }
      }
    }
  );

  // ターミナル追加レスポンス
  unlistenAddResponse = await appWindow.listen<{ terminalId: number; sessionId: number; title: string }>(
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
  );

  // メインウィンドウからのフォーカスリクエスト
  unlistenFocusTerminal = await appWindow.listen<{ terminalId: number }>(
    "sub-focus-terminal",
    async (event) => {
      const { terminalId } = event.payload;
      // ターミナルが属するリーフを探す
      const leafs = getLeafsWithTerminals();
      const leaf = leafs.find((l) => l.terminalIds.includes(terminalId));
      if (leaf) {
        await switchTerminal(leaf.id, terminalId);
      }
    }
  );

  // メインウィンドウが「メインに戻す」を選択
  unlistenClosingByMain = await appWindow.listen<{ worktreeId: string }>(
    "sub-closing-by-main",
    async (event) => {
      if (event.payload.worktreeId === worktreeId) {
        closingByMain = true;
        // 全ターミナルの PTY を kill
        for (const [, entry] of terminalEntries) {
          const termRef = terminalRefs.get(entry.id);
          if (termRef?.isRunning) {
            await termRef.kill();
          }
        }
        // kill 完了をメインに通知（destroy 前に送信）
        await emitTo("main", "sub-window-closed-ack", { worktreeId });
        await appWindow.destroy();
      }
    }
  );

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

  // サムネイル送信ループ（変化があった場合のみ送信）
  const lastThumbnailUrls = new Map<number, string>();
  thumbnailInterval = setInterval(() => {
    for (const [id] of terminalEntries) {
      const ref = terminalRefs.get(id);
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
  unlistenGetLayout = await appWindow.listen("sub-get-layout", async () => {
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
  });

  // 自動承認フラグ更新
  unlistenSetAutoApproval = await appWindow.listen<{ autoApproval: boolean }>(
    "sub-set-auto-approval",
    (event) => {
      autoApproval.value = event.payload.autoApproval;
    }
  );

  // 自動承認チェック（notify-worktree トリガー）
  unlistenTryAutoApprove = await appWindow.listen("sub-try-auto-approve", async () => {
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
    let approved = false;
    try {
      for (const [tid] of terminalEntries) {
        const termRef = terminalRefs.get(tid);
        if (!termRef) { await debug(`[AutoApproval] tid=${tid} termRef=null, skip`); continue; }
        const terminal = termRef.getTerminal();
        if (!terminal) { await debug(`[AutoApproval] tid=${tid} terminal=null, skip`); continue; }
        const content = getRecentLines(terminal, 200);
        await debug(`[AutoApproval] tid=${tid} content(last200)=${content.slice(-200)}`);
        if (await analyzeForApproval(worktreeId, content, worktreePath)) {
          // バッファ再チェック: AI判定完了後、承認プロンプトがまだあるか確認
          const freshContent = getRecentLines(terminal, 10);
          if (!hasApprovalPrompt(freshContent)) {
            await debug(`[AutoApproval] tid=${tid} → prompt disappeared, skip Enter`);
            break;
          }
          await debug(`[AutoApproval] tid=${tid} → approved, sending Enter`);
          await termRef.write("\r");
          approved = true;
          break;
        } else {
          await debug(`[AutoApproval] tid=${tid} → not approved`);
        }
      }
    } finally {
      aiJudging.value = false;
    }
    await debug(`[AutoApproval] sub result: approved=${approved}`);
    await emitTo("main", "sub-auto-approve-result", { worktreeId, approved });
  });

  // AI判定キャンセル
  unlistenCancelAutoApprove = await appWindow.listen("sub-cancel-auto-approve", async () => {
    await debug(`[AutoApproval] sub-cancel-auto-approve received`);
    await invoke("cancel_approval", { worktreeId });
    aiJudging.value = false;
  });

  // セッション保存リクエスト
  unlistenSessionSaveRequest = await appWindow.listen("sub-session-save-request", async () => {
    const terminals = Array.from(terminalEntries.values()).map((entry) => {
      const termRef = terminalRefs.get(entry.id);
      return { title: entry.title, buffer: termRef?.serializeBuffer() ?? "" };
    }).filter((t) => t.buffer !== "");
    await emitTo("main", "sub-session-save-response", { worktreeId, terminals });
  });

  // メインに準備完了を通知
  await emitTo("main", "sub-ready", { worktreeId });
});

onUnmounted(() => {
  window.removeEventListener("keydown", handleAltCharKey, true);
  if (thumbnailInterval) clearInterval(thumbnailInterval);
  unlistenInit?.();
  unlistenAddResponse?.();
  unlistenFocusTerminal?.();
  unlistenClosingByMain?.();
  unlistenGetLayout?.();
  unlistenSetAutoApproval?.();
  unlistenTryAutoApprove?.();
  unlistenCancelAutoApprove?.();
  unlistenSettingsChanged?.();
  unlistenSessionSaveRequest?.();
});

async function onCancelAiJudging() {
  await invoke("cancel_approval", { worktreeId });
  aiJudging.value = false;
}

function getFirstLeafId(): string {
  function find(node: import("./types/frame").FrameNode): string {
    if (node.type === "leaf") return node.id;
    return find(node.children[0]);
  }
  return find(root.value);
}
</script>

<template>
  <div class="h-screen flex flex-col bg-[#1e1e2e] text-[#cdd6f4] select-none">
    <!-- 初期化中 -->
    <div v-if="!initialized" class="flex items-center justify-center h-full text-[#6c7086] text-sm">
      {{ t('connecting') }}
    </div>

    <template v-else>
      <!-- ヘッダー -->
      <div 
        class="flex items-center justify-between border-b shrink-0 px-4 py-1 transition-colors duration-200"
        :class="isWindowFocused ? 'bg-gradient-to-r from-[#181825] via-[#2a2a3f] to-[#181825] animate-gradient-x border-[#cba6f7]/50' : 'bg-[#11111b] opacity-80 border-[#313244]'"
      >
        <div class="flex items-center gap-2">
          <span
            class="text-sm font-semibold transition-colors duration-200"
            :class="isWindowFocused ? 'text-[#cba6f7]' : 'text-[#6c7086]'"
          >
            {{ worktreeName }}
          </span>
          <span
            v-if="hotkeyChar"
            class="text-[10px] px-1.5 py-0.5 rounded font-mono font-medium"
            style="background: rgba(203,166,247,0.15); color: #cba6f7; border: 1px solid rgba(203,166,247,0.3)"
          >Alt+{{ hotkeyChar.toUpperCase() }}</span>
          <span
            v-if="autoApproval"
            class="text-[10px] px-1.5 py-0.5 rounded font-medium"
            style="background: rgba(166, 227, 161, 0.15); color: #a6e3a1; border: 1px solid rgba(166, 227, 161, 0.3)"
          >{{ t('autoApprovalBadge') }}</span>
          <button
            v-if="aiJudging"
            class="flex items-center gap-1 text-[10px] px-1.5 py-0.5 rounded font-semibold cursor-pointer border-none"
            style="background: #f9e2af; color: #1e1e2e"
            @click="onCancelAiJudging"
          >
            <span class="pi pi-spin pi-spinner" style="font-size: 9px" />
            {{ t('aiJudgingBadge') }}
          </button>
        </div>
        <button
          class="flex items-center justify-center w-7 h-7 rounded bg-[#313244] hover:bg-[#45475a] text-[#cdd6f4] transition-colors"
          :title="t('openInIde')"
          @click="requestOpenInIde"
        >
          <span class="pi pi-code text-sm" />
        </button>
      </div>

      <!-- フレームレイアウト -->
      <div class="flex-1 min-h-0 overflow-hidden">
        <FrameContainer
          :node="root"
          :terminal-entries="terminalEntries"
          :terminal-exit-codes="terminalExitCodes"
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
    "autoApprovalBadge": "Auto approval",
    "aiJudgingBadge": "AI judging",
    "openInIde": "Open in IDE",
    "ideNotInstalled": "None of Cursor, VS Code, Antigravity are installed.",
    "ideNotInstalledTitle": "IDE not found",
    "ideLaunchFailed": "Failed to launch IDE: {error}"
  },
  "ja": {
    "connecting": "接続中...",
    "autoApprovalBadge": "自動承認",
    "aiJudgingBadge": "AI判定中",
    "openInIde": "IDE で開く",
    "ideNotInstalled": "Cursor、VS Code、Antigravity のいずれもインストールされていません。",
    "ideNotInstalledTitle": "IDE が見つかりません",
    "ideLaunchFailed": "IDE の起動に失敗しました: {error}"
  }
}
</i18n>
