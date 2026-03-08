<script setup lang="ts">
import { ref, reactive, onMounted, onUnmounted, nextTick, markRaw, computed } from "vue";
import { emitTo, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import TerminalView from "./components/TerminalView.vue";
import FrameContainer from "./components/FrameContainer.vue";
import { useFrameLayout } from "./composables/useFrameLayout";
import { useSettings } from "./composables/useSettings";
import { useHotkeyListener } from "./composables/useHotkeys";
import type { TrayWorktreeData } from "./composables/useTrayPopup";
import type { FrameNode } from "./types/frame";
import { useI18n } from "vue-i18n";

const { t } = useI18n();

interface TrayTerminalEntry {
  id: number;
  title: string;
  sessionId: number;
  snapshot: string;
  rows: number;
  cols: number;
}

// フッター ref（ウィンドウサイズ補正用）
const footerRef = ref<HTMLDivElement | null>(null);

// 全ワークツリーデータ
const allWorktrees = ref<TrayWorktreeData[]>([]);
const currentIndex = ref(0);
const initialized = ref(false);

// フレームレイアウト
const { root, initLayout, removeTerminalFromLeaf, moveTerminal, setActiveTerminal, splitLeaf, pruneTree, findLeafByTerminalId, getAllLeafs } = useFrameLayout();

// ターミナルエントリ（現在のワークツリーのみ）
const terminalEntries = reactive(new Map<number, TrayTerminalEntry>());
const lastFocusedLeafId = ref<string>("");

// TerminalView ref 管理
const terminalRefs = reactive(new Map<number, InstanceType<typeof TerminalView>>());

function setTerminalRef(terminalId: number, el: unknown) {
  if (el) {
    terminalRefs.set(terminalId, markRaw(el as InstanceType<typeof TerminalView>));
  } else {
    terminalRefs.delete(terminalId);
  }
}

// ────────────────────────────────────────────────
// DOM reparenting
// ────────────────────────────────────────────────

function returnAllToOffscreen(): void {
  const offscreen = document.querySelector('[data-offscreen]');
  if (!offscreen) return;
  for (const [tid] of terminalEntries) {
    const comp = terminalRefs.get(tid);
    const el = comp?.containerRef;
    if (el && el.parentElement !== offscreen) {
      offscreen.appendChild(el);
    }
  }
}

function mountTerminalsToHosts(): void {
  for (const [tid] of terminalEntries) {
    const comp = terminalRefs.get(tid);
    const el = comp?.containerRef;
    const host = document.getElementById(`terminal-host-${tid}`);
    if (el && host && el.parentElement !== host) {
      host.appendChild(el);
    }
  }
}

// ────────────────────────────────────────────────
// 現在ワークツリーの表示
// ────────────────────────────────────────────────

async function showWorktree(data: TrayWorktreeData) {
  terminalEntries.clear();
  terminalRefs.clear();

  // ウィンドウサイズをサブウィンドウに合わせる（フッター高さ分を加算）
  const win = getCurrentWindow();
  const footerH = footerRef.value?.offsetHeight ?? 0;
  const width = data.windowSize?.width ?? 900;
  const height = (data.windowSize?.height ?? 600) + footerH;
  await win.setSize(new LogicalSize(width, height));

  for (const t of data.terminals) {
    terminalEntries.set(t.id, { ...t });
  }

  const ids = data.terminals.map((t) => t.id);

  // レイアウト復元: detached でレイアウトがある場合はそのまま設定
  if (data.isDetached && data.layout) {
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
  mountTerminalsToHosts();

  // 最初のアクティブターミナルにフォーカス
  const firstLeaf = leafs[0];
  if (firstLeaf?.activeTerminalId !== null && firstLeaf?.activeTerminalId !== undefined) {
    const term = terminalRefs.get(firstLeaf.activeTerminalId);
    if (term) {
      await term.handleTabActivated();
      // PTYサイズに合わせてxterm.jsをリサイズ（noResize=trueのためfit()は呼ばれない）
      const entry = terminalEntries.get(firstLeaf.activeTerminalId);
      if (entry) {
        const termObj = term.getTerminal();
        if (termObj) {
          termObj.resize(entry.cols, entry.rows);
          termObj.refresh(0, termObj.rows - 1);
          termObj.scrollToBottom();
        }
      }
      term.focus();
    }
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
    // PTYサイズに合わせてxterm.jsをリサイズ（noResize=trueのためfit()は呼ばれない）
    const entry = terminalEntries.get(terminalId);
    if (entry) {
      const termObj = term.getTerminal();
      if (termObj) {
        termObj.resize(entry.cols, entry.rows);
        termObj.refresh(0, termObj.rows - 1);
        termObj.scrollToBottom();
      }
    }
    term.focus();
  }
}

function handleTerminalExit(tid: number) {
  const leaf = findLeafByTerminalId(tid);
  if (leaf) closeTerminal(leaf.id, tid);
}

async function closeTerminal(leafId: string, terminalId: number) {
  if (!terminalEntries.has(terminalId)) return;

  const term = terminalRefs.get(terminalId);
  if (term?.isRunning) {
    await term.kill();
  }

  if (!terminalEntries.has(terminalId)) return;

  returnAllToOffscreen();

  terminalEntries.delete(terminalId);
  removeTerminalFromLeaf(leafId, terminalId);
  pruneTree();

  await nextTick();
  mountTerminalsToHosts();

  for (const [tid] of terminalEntries) {
    const t = terminalRefs.get(tid);
    if (t) await t.handleTabActivated();
  }
}

async function onSplitRequest(leafId: string, direction: "left" | "right" | "top" | "bottom") {
  returnAllToOffscreen();
  splitLeaf(leafId, direction);
  lastFocusedLeafId.value = leafId;
  await nextTick();
  mountTerminalsToHosts();
  for (const [tid] of terminalEntries) {
    const t = terminalRefs.get(tid);
    if (t) await t.handleTabActivated();
  }
}

function onTabReorder(leafId: string, terminalId: number, insertIndex: number) {
  moveTerminal(terminalId, leafId, leafId, insertIndex);
}

async function onTabDrop(sourceLeafId: string, terminalId: number, targetLeafId: string, insertIndex?: number) {
  if (sourceLeafId === targetLeafId) return;
  returnAllToOffscreen();
  moveTerminal(terminalId, sourceLeafId, targetLeafId, insertIndex);
  pruneTree();
  lastFocusedLeafId.value = targetLeafId;
  await nextTick();
  mountTerminalsToHosts();
  for (const [tid] of terminalEntries) {
    const t = terminalRefs.get(tid);
    if (t) await t.handleTabActivated();
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
    const t = terminalRefs.get(tid);
    if (t) await t.handleTabActivated();
  }
  const movedTerm = terminalRefs.get(terminalId);
  if (movedTerm) movedTerm.focus();
}

// ────────────────────────────────────────────────
// ナビゲーション
// ────────────────────────────────────────────────

const currentWorktree = computed(() => allWorktrees.value[currentIndex.value] ?? null);
const isLast = computed(() => currentIndex.value >= allWorktrees.value.length - 1);

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
    await showWorktree(allWorktrees.value[currentIndex.value]);
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
  await detachCurrentTerminals();
  await emitTo("main", "tray-closing", {});
  await getCurrentWindow().destroy();
}

function onHeaderDrag(e: MouseEvent) {
  if ((e.target as HTMLElement).closest('button')) return
  getCurrentWindow().startDragging()
}

// ────────────────────────────────────────────────
// ライフサイクル
// ────────────────────────────────────────────────

const { settings, loadSettings } = useSettings();

// TrayPopup のホットキーリスナー (trayNext)
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
  <div class="h-screen flex flex-col bg-[#1e1e2e] text-[#cdd6f4] select-none">
    <!-- ヘッダー (drag-region) -->
    <div
      class="flex items-center justify-between bg-[#181825] border-b border-[#313244] shrink-0 px-4 py-2"
      @mousedown.left="onHeaderDrag"
    >
      <div class="flex items-center gap-3 pointer-events-none">
        <span class="pi pi-bell text-[#cba6f7]" />
        <span class="text-sm font-semibold text-[#cba6f7]">
          {{ currentWorktree?.worktreeName ?? t('tray.notification') }}
        </span>
        <span
          v-if="allWorktrees.length > 1"
          class="text-xs text-[#6c7086]"
        >
          {{ currentIndex + 1 }} / {{ allWorktrees.length }}
        </span>
      </div>
      <button
        class="pointer-events-auto w-6 h-6 flex items-center justify-center rounded hover:bg-[#313244] text-[#6c7086] hover:text-[#f38ba8] transition-colors"
        :title="t('tray.close')"
        @click="onClose"
      >
        <span class="pi pi-times text-xs" />
      </button>
    </div>

    <!-- コンテンツ -->
    <div class="flex-1 min-h-0 overflow-hidden">
      <div v-if="!initialized" class="flex items-center justify-center h-full text-[#6c7086] text-sm">
        {{ t('tray.loading') }}
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
        {{ t('tray.noTerminals') }}
      </div>
    </div>

    <!-- フッター -->
    <div ref="footerRef" class="flex items-center justify-end gap-2 bg-[#181825] border-t border-[#313244] shrink-0 px-4 py-2">
      <button
        v-if="!isLast"
        class="px-4 py-1.5 text-sm rounded bg-[#313244] hover:bg-[#45475a] text-[#cdd6f4] transition-colors"
        @click="onNext"
      >
        {{ t('tray.next') }}
      </button>
      <button
        class="px-4 py-1.5 text-sm rounded bg-[#a6e3a1] hover:bg-[#89c98a] text-[#1e1e2e] font-semibold transition-colors"
        @click="onDone"
      >
        {{ t('tray.done') }}
      </button>
    </div>

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
          @exit="handleTerminalExit(tid)"
          @title-change="() => {}"
        />
      </template>
    </div>
  </div>
</template>
