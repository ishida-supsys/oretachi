<script setup lang="ts">
import { ref, reactive, onMounted, onUnmounted, nextTick, computed } from "vue";
import { emitTo, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import TerminalView from "./components/TerminalView.vue";
import FrameContainer from "./components/FrameContainer.vue";
import IdeSelectDialog from "./components/IdeSelectDialog.vue";
import { useWorktreeFrame } from "./composables/useWorktreeFrame";
import { useSettings } from "./composables/useSettings";
import { useHotkeyListener } from "./composables/useHotkeys";
import { useIdeSelect } from "./composables/useIdeSelect";
import type { TrayWorktreeData } from "./composables/useTrayPopup";
import type { FrameNode } from "./types/frame";
import type { TrayTerminalEntry } from "./types/terminal";
import { useI18n } from "vue-i18n";

const { t } = useI18n();

// ヘッダー ref（ウィンドウサイズ補正用）
const headerRef = ref<HTMLDivElement | null>(null);
// フッター ref（ウィンドウサイズ補正用）
const footerRef = ref<HTMLDivElement | null>(null);

// 全ワークツリーデータ
const allWorktrees = ref<TrayWorktreeData[]>([]);
const currentIndex = ref(0);
const initialized = ref(false);

// ターミナルエントリ（現在のワークツリーのみ）
const terminalEntries = reactive(new Map<number, TrayTerminalEntry>());

// TerminalView ref 管理
const terminalRefs = reactive(new Map<number, InstanceType<typeof TerminalView>>());

// フレームレイアウト（useWorktreeFrameで共通化）
const {
  root,
  initLayout,
  lastFocusedLeafId,
  setTerminalRef,
  returnAllToOffscreen,
  mountTerminalsToHosts,
  getAllLeafs,
  switchTerminal,
  closeTerminal,
  handleTerminalExit,
  onSplitRequest,
  onTabDrop,
  onTabEdgeDrop,
  onTabReorder,
  switchNextTerminal,
  switchPrevTerminal,
  closeActiveTerminal,
} = useWorktreeFrame({
  terminalEntries,
  terminalRefs,
  // noResize=true のためタブ切替時に PTY サイズを手動適用
  async onAfterSwitch(_leafId, terminalId) {
    const entry = terminalEntries.get(terminalId);
    const term = terminalRefs.get(terminalId);
    if (entry && term) {
      const termObj = term.getTerminal();
      if (termObj) {
        termObj.resize(entry.cols, entry.rows);
        termObj.refresh(0, termObj.rows - 1);
        termObj.scrollToBottom();
      }
    }
  },
});

// ────────────────────────────────────────────────
// 現在ワークツリーの表示
// ────────────────────────────────────────────────

async function showWorktree(data: TrayWorktreeData) {
  terminalEntries.clear();
  terminalRefs.clear();

  // ウィンドウサイズをサブウィンドウに合わせる
  // isDetached=true: windowSize はサブウィンドウ全体のサイズ → フッターのみ加算
  // isDetached=false: windowSize はメインウィンドウのフレーム領域 → ヘッダー + フッター加算
  const win = getCurrentWindow();
  const footerH = footerRef.value?.offsetHeight ?? 0;
  const headerH = data.isDetached ? 0 : (headerRef.value?.offsetHeight ?? 0);
  const width = data.windowSize?.width ?? 900;
  const height = (data.windowSize?.height ?? 600) + footerH + headerH;
  await win.setSize(new LogicalSize(width, height));

  for (const t of data.terminals) {
    terminalEntries.set(t.id, { ...t });
  }

  const ids = data.terminals.map((t) => t.id);

  // レイアウト復元: layout があればそのまま設定（detached/non-detached 両対応）
  if (data.layout) {
    root.value = data.layout as FrameNode;
  } else {
    initLayout(ids);
  }

  // 最初のリーフを lastFocusedLeafId に設定
  const leafs = getAllLeafs();
  if (leafs.length > 0) {
    lastFocusedLeafId.value = leafs[0].id;
  }

  await nextTick();
  // OSレベルのウィンドウリサイズがレイアウトに反映されるのを待つ
  await new Promise<void>((resolve) => {
    requestAnimationFrame(() => {
      requestAnimationFrame(() => resolve());
    });
  });
  mountTerminalsToHosts();

  // 全リーフのアクティブターミナルをリサイズ+リフレッシュ
  for (const leaf of leafs) {
    if (leaf.activeTerminalId == null) continue;
    const term = terminalRefs.get(leaf.activeTerminalId);
    if (!term) continue;
    await term.handleTabActivated();
    // PTYサイズに合わせてxterm.jsをリサイズ（noResize=trueのためfit()は呼ばれない）
    const entry = terminalEntries.get(leaf.activeTerminalId);
    if (entry) {
      const termObj = term.getTerminal();
      if (termObj) {
        termObj.resize(entry.cols, entry.rows);
        termObj.refresh(0, termObj.rows - 1);
        termObj.scrollToBottom();
      }
    }
  }

  // 最初のリーフにフォーカス
  const firstActiveId = leafs[0]?.activeTerminalId;
  if (firstActiveId != null) {
    terminalRefs.get(firstActiveId)?.focus();
  }
}

// ────────────────────────────────────────────────
// イベントハンドラ
// ────────────────────────────────────────────────

// ────────────────────────────────────────────────
// ナビゲーション
// ────────────────────────────────────────────────

const currentWorktree = computed(() => allWorktrees.value[currentIndex.value] ?? null);
const isLast = computed(() => currentIndex.value >= allWorktrees.value.length - 1);

// IDE で開く
const { showIdeDialog, detectedIdes, openInIde, onIdeSelected } = useIdeSelect();

async function onOpenInIde() {
  const wt = currentWorktree.value;
  if (!wt) return;
  await openInIde(wt.worktreePath, { worktreeId: wt.worktreeId, worktreeName: wt.worktreeName, origin: "tray" });
}

/** 現在のターミナルを detach してから次に進む */
async function detachCurrentTerminals() {
  returnAllToOffscreen();
  for (const [, ref] of terminalRefs) {
    ref.detach();
  }
  await nextTick();
}

async function onNext() {
  const wt = currentWorktree.value;
  if (!wt) return;

  await detachCurrentTerminals();

  // 通知クリアをメインに通知
  await emitTo("main", "tray-clear-notification", { worktreeId: wt.worktreeId });

  currentIndex.value += 1;
  if (currentIndex.value < allWorktrees.value.length) {
    const next = allWorktrees.value[currentIndex.value];
    await emitTo("main", "tray-current-worktree-changed", { worktreeId: next.worktreeId });
    await showWorktree(next);
  }
}

async function onDone() {
  const wt = currentWorktree.value;
  if (wt) {
    await detachCurrentTerminals();
    await emitTo("main", "tray-clear-notification", { worktreeId: wt.worktreeId });
  }
  await emitTo("main", "tray-closing", {});
  await getCurrentWindow().destroy();
}

async function onClose() {
  const wt = currentWorktree.value;
  if (wt) {
    await emitTo("main", "tray-clear-notification", { worktreeId: wt.worktreeId });
  }
  await detachCurrentTerminals();
  await emitTo("main", "tray-closing", {});
  await getCurrentWindow().destroy();
}

function onHeaderDrag(e: MouseEvent) {
  if ((e.target as HTMLElement).closest('button')) return
  getCurrentWindow().startDragging()
}

async function onClickAutoApproval() {
  const wt = currentWorktree.value;
  if (!wt) return;
  await emitTo("main", "tray-click-auto-approval", { worktreeId: wt.worktreeId });
}

async function onCancelAiJudging() {
  const wt = currentWorktree.value;
  if (!wt) return;
  await emitTo("main", "tray-cancel-ai-judging", { worktreeId: wt.worktreeId });
}

// ────────────────────────────────────────────────
// ライフサイクル
// ────────────────────────────────────────────────

const { settings, loadSettings } = useSettings();

// TrayPopup のホットキーリスナー
useHotkeyListener(() => {
  const hk = settings.value.hotkeys;
  if (!hk || !initialized.value) return [];
  return [
    {
      binding: hk.trayNext,
      handler: () => {
        if (isLast.value) {
          onDone();
        } else {
          onNext();
        }
      },
    },
    { binding: hk.terminalNext, handler: switchNextTerminal },
    { binding: hk.terminalPrev, handler: switchPrevTerminal },
    { binding: hk.terminalClose, handler: closeActiveTerminal },
  ];
});

let unlistenInit: UnlistenFn | null = null;

onMounted(async () => {
  await loadSettings();

  const appWindow = getCurrentWindow();

  unlistenInit = await appWindow.listen<{ worktrees: TrayWorktreeData[] }>(
    "tray-init",
    async (event) => {
      allWorktrees.value = event.payload.worktrees;
      currentIndex.value = 0;
      initialized.value = true;

      if (allWorktrees.value.length > 0) {
        await showWorktree(allWorktrees.value[0]);
      }
    }
  );

  // 準備完了をメインに通知
  await emitTo("main", "tray-ready", {});
});

onUnmounted(() => {
  unlistenInit?.();
});
</script>

<template>
  <div class="h-screen flex flex-col text-[#cdd6f4] select-none" style="background-color: var(--bg-base)">
    <!-- ヘッダー (drag-region) -->
    <div
      ref="headerRef"
      class="flex items-center justify-between border-b border-[#313244] shrink-0 px-4 py-2"
      style="background-color: var(--bg-mantle-translucent)"
      @mousedown.left="onHeaderDrag"
    >
      <div class="flex items-center gap-2">
        <span class="pi pi-bell text-[#cba6f7] pointer-events-none" />
        <span class="text-sm font-semibold text-[#cba6f7] pointer-events-none">
          {{ currentWorktree?.worktreeName ?? t('notification') }}
        </span>
        <span
          v-if="allWorktrees.length > 1"
          class="text-xs text-[#6c7086] pointer-events-none"
        >
          {{ currentIndex + 1 }} / {{ allWorktrees.length }}
        </span>
        <span
          v-if="currentWorktree?.branchName"
          class="flex items-center gap-1 text-xs font-mono text-[#9399b2] pointer-events-none"
        >
          <span class="pi pi-code-branch" style="font-size: 10px" />
          {{ currentWorktree.branchName }}
        </span>
        <span
          v-if="currentWorktree?.hotkeyChar"
          class="text-[10px] px-1.5 py-0.5 rounded font-mono font-medium pointer-events-none"
          style="background: rgba(203,166,247,0.15); color: #cba6f7; border: 1px solid rgba(203,166,247,0.3)"
        >Alt+{{ currentWorktree.hotkeyChar.toUpperCase() }}</span>
        <button
          v-if="currentWorktree?.autoApproval"
          class="text-[10px] px-1.5 py-0.5 rounded font-medium cursor-pointer border-none"
          style="background: rgba(166, 227, 161, 0.15); color: #a6e3a1; border: 1px solid rgba(166, 227, 161, 0.3)"
          :title="t('editAutoApprovalPrompt')"
          @click="onClickAutoApproval"
        >{{ t('autoApprovalBadge') }}</button>
        <button
          v-if="currentWorktree?.aiJudging"
          class="flex items-center gap-1 text-[10px] px-1.5 py-0.5 rounded font-semibold cursor-pointer border-none"
          style="background: #f9e2af; color: #1e1e2e"
          @click="onCancelAiJudging"
        >
          <span class="pi pi-spin pi-spinner" style="font-size: 9px" />
          {{ t('aiJudgingBadge') }}
        </button>
      </div>
      <div class="flex items-center gap-4">
        <button
          v-if="currentWorktree"
          class="pointer-events-auto w-6 h-6 flex items-center justify-center rounded hover:bg-[#313244] text-[#6c7086] hover:text-[#cdd6f4] transition-colors"
          :title="t('openInIde')"
          @click="onOpenInIde"
        >
          <span class="pi pi-code text-xs" />
        </button>
        <button
          class="pointer-events-auto w-6 h-6 flex items-center justify-center rounded hover:bg-[#313244] text-[#6c7086] hover:text-[#f38ba8] transition-colors"
          :title="t('close')"
          @click="onClose"
        >
          <span class="pi pi-times text-xs" />
        </button>
      </div>
    </div>

    <!-- コンテンツ -->
    <div class="flex-1 min-h-0 overflow-hidden">
      <div v-if="!initialized" class="flex items-center justify-center h-full text-[#6c7086] text-sm">
        {{ t('loading') }}
      </div>

      <FrameContainer
        v-else-if="terminalEntries.size > 0"
        :node="root"
        :terminal-entries="terminalEntries"
        @switch-terminal="switchTerminal"
        @close-terminal="closeTerminal"
        @title-change="() => {}"
        @split-request="onSplitRequest"
        @tab-drop="onTabDrop"
        @tab-edge-drop="onTabEdgeDrop"
        @tab-reorder="onTabReorder"
        @request-add-terminal="() => {}"
        @resize-end="() => {}"
      />

      <div v-else-if="initialized" class="flex items-center justify-center h-full text-[#6c7086] text-sm">
        {{ t('noTerminals') }}
      </div>
    </div>

    <!-- フッター -->
    <div ref="footerRef" class="flex items-center justify-end gap-2 border-t border-[#313244] shrink-0 px-4 py-2" style="background-color: var(--bg-mantle-translucent)">
      <button
        v-if="!isLast"
        class="px-4 py-1.5 text-sm rounded bg-[#313244] hover:bg-[#45475a] text-[#cdd6f4] transition-colors"
        @click="onNext"
      >
        {{ t('next') }}
      </button>
      <button
        class="px-4 py-1.5 text-sm rounded bg-[#a6e3a1] hover:bg-[#89c98a] text-[#1e1e2e] font-semibold transition-colors"
        @click="onDone"
      >
        {{ t('done') }}
      </button>
    </div>

    <!-- IDE 選択ダイアログ -->
    <IdeSelectDialog
      v-if="showIdeDialog"
      :ides="detectedIdes"
      @select="onIdeSelected"
      @cancel="showIdeDialog = false"
    />

    <!-- TerminalView のマウント先 -->
    <div
      data-offscreen
      style="position:fixed; left:-10000px; top:-10000px; width:1000px; height:1000px; overflow:hidden; pointer-events:none"
    >
      <template v-for="[tid, entry] in terminalEntries" :key="tid">
        <TerminalView
          :ref="(el) => setTerminalRef(tid, el)"
          :no-resize="true"
          :auto-start="false"
          :initial-session-id="entry.sessionId"
          :initial-snapshot="entry.snapshot"
          :initial-cols="entry.cols"
          :initial-rows="entry.rows"
          @exit="handleTerminalExit(tid)"
          @title-change="() => {}"
        />
      </template>
    </div>
  </div>
</template>

<i18n lang="json">
{
  "en": {
    "notification": "Notification",
    "close": "Close",
    "openInIde": "Open in IDE",
    "loading": "Loading...",
    "noTerminals": "No terminals",
    "next": "Next →",
    "done": "Done ✓",
    "autoApprovalBadge": "Auto approval",
    "aiJudgingBadge": "AI judging",
    "editAutoApprovalPrompt": "Edit additional prompt"
  },
  "ja": {
    "notification": "通知",
    "close": "閉じる",
    "openInIde": "IDE で開く",
    "loading": "読み込み中...",
    "noTerminals": "ターミナルがありません",
    "next": "次へ →",
    "done": "完了 ✓",
    "autoApprovalBadge": "自動承認",
    "aiJudgingBadge": "AI判定中",
    "editAutoApprovalPrompt": "追加プロンプトを編集"
  }
}
</i18n>
