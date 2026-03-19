<script lang="ts">
// モジュールレベル: 全 FramePane インスタンス間で共有するドラッグ状態フラグ
// WebView2 では dragover 中に dataTransfer.types にカスタム MIME タイプが含まれないため
// types チェックの代わりにこのフラグを使う
let isDraggingTab = false;
</script>

<script setup lang="ts">
import { ref, computed } from "vue";
import type { FrameLeaf } from "../types/frame";
import type { SubTerminalEntry } from "../types/terminal";
import { useI18n } from "vue-i18n";

const { t } = useI18n();

// ─── タブバーD&D ───────────────────────────────────────────────
const tabDropIndicatorIndex = ref<number | null>(null);
const tabDropIndicatorLeft = ref(0);
const tabsScrollRef = ref<HTMLElement | null>(null);

const tabDropIndicatorStyle = computed(() => ({
  left: `${tabDropIndicatorLeft.value}px`,
}));

const props = defineProps<{
  leaf: FrameLeaf;
  terminalEntries: Map<number, SubTerminalEntry>;
  terminalExitCodes?: Map<number, number>;
  terminalAgentStatus?: Map<number, boolean>;
}>();

const emit = defineEmits<{
  switchTerminal: [leafId: string, terminalId: number];
  closeTerminal: [leafId: string, terminalId: number];
  titleChange: [terminalId: number, title: string];
  splitRequest: [leafId: string, direction: "left" | "right" | "top" | "bottom"];
  tabDrop: [sourceLeafId: string, terminalId: number, targetLeafId: string, insertIndex?: number];
  tabEdgeDrop: [sourceLeafId: string, terminalId: number, targetLeafId: string, direction: "left" | "right" | "top" | "bottom"];
  tabReorder: [leafId: string, terminalId: number, insertIndex: number];
  requestAddTerminal: [leafId: string];
}>();

// ドロップゾーン表示
type DropZone = "left" | "right" | "top" | "bottom" | "center" | null;
const dropZone = ref<DropZone>(null);
const isDragOver = ref(false);

function computeDropZone(event: DragEvent, el: HTMLElement): DropZone {
  const rect = el.getBoundingClientRect();
  const x = event.clientX - rect.left;
  const y = event.clientY - rect.top;
  const w = rect.width;
  const h = rect.height;
  const edgeRatio = 0.2;

  if (x < w * edgeRatio) return "left";
  if (x > w * (1 - edgeRatio)) return "right";
  if (y < h * edgeRatio) return "top";
  if (y > h * (1 - edgeRatio)) return "bottom";
  return "center";
}

function onTabDragStart(event: DragEvent, terminalId: number) {
  if (!event.dataTransfer) return;
  event.dataTransfer.setData("x-terminal-id", String(terminalId));
  event.dataTransfer.setData("x-source-leaf-id", props.leaf.id);
  event.dataTransfer.effectAllowed = "move";
  isDraggingTab = true;
}

function onTabDragEnd() {
  isDraggingTab = false;
}

const paneRef = ref<HTMLElement | null>(null);

function onDragOver(event: DragEvent) {
  if (!isDraggingTab) return;
  event.preventDefault();
  if (event.dataTransfer) event.dataTransfer.dropEffect = "move";

  if (paneRef.value) {
    dropZone.value = computeDropZone(event, paneRef.value);
  }
  isDragOver.value = true;
}

function onDragLeave(event: DragEvent) {
  // paneRef の外に出た時のみリセット（子要素への移動では消さない）
  if (paneRef.value && !paneRef.value.contains(event.relatedTarget as Node)) {
    dropZone.value = null;
    isDragOver.value = false;
  }
}

function onDrop(event: DragEvent) {
  event.preventDefault();
  isDraggingTab = false;
  const terminalId = Number(event.dataTransfer?.getData("x-terminal-id"));
  const sourceLeafId = event.dataTransfer?.getData("x-source-leaf-id") ?? "";
  const zone = dropZone.value;

  dropZone.value = null;
  isDragOver.value = false;

  if (!terminalId || !sourceLeafId) return;

  if (zone === "center" || zone === null) {
    if (sourceLeafId !== props.leaf.id) {
      emit("tabDrop", sourceLeafId, terminalId, props.leaf.id);
    }
  } else {
    emit("tabEdgeDrop", sourceLeafId, terminalId, props.leaf.id, zone);
  }
}

// ─── タブバーD&Dハンドラ ────────────────────────────────────────

/**
 * マウスX座標から各タブボタンの中央を比較して挿入インデックスを算出する。
 * 各タブの中央より左なら i、右なら i+1 を返す。
 */
function computeTabInsertIndex(event: DragEvent): number {
  if (!tabsScrollRef.value) return props.leaf.terminalIds.length;
  const tabs = Array.from(tabsScrollRef.value.querySelectorAll<HTMLElement>(".tab-button"));
  const scrollRect = tabsScrollRef.value.getBoundingClientRect();
  for (let i = 0; i < tabs.length; i++) {
    const rect = tabs[i].getBoundingClientRect();
    if (event.clientX < rect.left + rect.width / 2) {
      tabDropIndicatorLeft.value = rect.left - scrollRect.left;
      return i;
    }
  }
  // 末尾
  if (tabs.length > 0) {
    const last = tabs[tabs.length - 1].getBoundingClientRect();
    tabDropIndicatorLeft.value = last.right - scrollRect.left;
  } else {
    tabDropIndicatorLeft.value = 0;
  }
  return tabs.length;
}

function onTabBarDragOver(event: DragEvent) {
  if (!isDraggingTab) return;
  event.preventDefault();
  event.stopPropagation(); // ペイン全体の dragover を抑制
  if (event.dataTransfer) event.dataTransfer.dropEffect = "move";
  tabDropIndicatorIndex.value = computeTabInsertIndex(event);
  // ペインレベルの分割オーバーレイを消す
  isDragOver.value = false;
  dropZone.value = null;
}

function onTabBarDragLeave(event: DragEvent) {
  const related = event.relatedTarget as Node | null;
  if (!related) return; // WebView2: null の場合はリセットしない
  if (tabsScrollRef.value && !tabsScrollRef.value.contains(related)) {
    tabDropIndicatorIndex.value = null;
  }
}

function onTabBarDrop(event: DragEvent) {
  event.preventDefault();
  event.stopPropagation(); // ペイン全体の drop を抑制
  isDraggingTab = false;

  const terminalId = Number(event.dataTransfer?.getData("x-terminal-id"));
  const sourceLeafId = event.dataTransfer?.getData("x-source-leaf-id") ?? "";
  const insertIndex = computeTabInsertIndex(event);

  tabDropIndicatorIndex.value = null;
  dropZone.value = null;
  isDragOver.value = false;

  if (!terminalId || !sourceLeafId) return;

  if (sourceLeafId === props.leaf.id) {
    emit("tabReorder", props.leaf.id, terminalId, insertIndex);
  } else {
    emit("tabDrop", sourceLeafId, terminalId, props.leaf.id, insertIndex);
  }
}

// ドロップオーバーレイのスタイル計算
function overlayStyle(zone: DropZone): Record<string, string> {
  const base: Record<string, string> = {
    position: "absolute",
    backgroundColor: "rgba(137, 180, 250, 0.25)",
    pointerEvents: "none",
    zIndex: "100",
    borderRadius: "4px",
    border: "2px solid rgba(137, 180, 250, 0.8)",
    transition: "all 0.1s ease",
  };
  switch (zone) {
    case "left":   return { ...base, top: "0", left: "0", width: "50%", height: "100%" };
    case "right":  return { ...base, top: "0", right: "0", left: "50%", height: "100%" };
    case "top":    return { ...base, top: "0", left: "0", width: "100%", height: "50%" };
    case "bottom": return { ...base, bottom: "0", left: "0", width: "100%", top: "50%", height: "50%" };
    case "center": return { ...base, top: "0", left: "0", width: "100%", height: "100%", backgroundColor: "rgba(137, 180, 250, 0.15)" };
    default:       return {};
  }
}
</script>

<template>
  <div class="frame-pane" ref="paneRef" @dragover="onDragOver" @dragleave="onDragLeave" @drop="onDrop">
    <!-- タブバー -->
    <div class="tab-bar">
      <div
        ref="tabsScrollRef"
        class="tabs-scroll"
        @dragover="onTabBarDragOver"
        @dragleave="onTabBarDragLeave"
        @drop="onTabBarDrop"
      >
        <button
          v-for="tid in leaf.terminalIds"
          :key="tid"
          class="tab-button"
          :class="tid === leaf.activeTerminalId ? 'tab-active' : 'tab-inactive'"
          draggable="true"
          @dragstart="onTabDragStart($event, tid)"
          @dragend="onTabDragEnd"
          @click="emit('switchTerminal', leaf.id, tid)"
        >
          <span class="tab-title">{{ terminalEntries.get(tid)?.title ?? `Terminal ${tid}` }}</span>
          <span
            v-if="terminalAgentStatus?.get(tid)"
            class="pi pi-microchip text-[10px] text-[#a6e3a1] shrink-0"
            title="AI Agent"
          />
          <span
            v-else-if="terminalExitCodes?.has(tid)"
            class="w-2 h-2 rounded-full inline-block shrink-0"
            :class="terminalExitCodes.get(tid) === 0 ? 'bg-[#89b4fa]' : 'bg-[#f38ba8]'"
          />
          <span
            class="pi pi-times tab-close"
            @click.stop="emit('closeTerminal', leaf.id, tid)"
          />
        </button>
        <!-- 単一インジケータ（表示/非表示は v-show、位置は left で制御） -->
        <div
          v-show="tabDropIndicatorIndex !== null"
          class="tab-drop-indicator"
          :style="tabDropIndicatorStyle"
        />
      </div>

      <button
        class="add-button"
        :title="t('addTerminal')"
        @click="emit('requestAddTerminal', leaf.id)"
      >+</button>
    </div>

    <!-- ターミナル群 -->
    <div class="terminal-area">
      <div
        v-for="tid in leaf.terminalIds"
        :key="tid"
        v-show="tid === leaf.activeTerminalId"
        class="terminal-slot"
      >
        <div :id="`terminal-host-${tid}`" class="terminal-teleport-target" />
      </div>

      <div v-if="leaf.terminalIds.length === 0" class="empty-pane">
        {{ t('noTerminals') }}
      </div>

      <!-- ドロップゾーンオーバーレイ -->
      <div v-if="isDragOver && dropZone" :style="overlayStyle(dropZone)" />
    </div>
  </div>
</template>

<style scoped>
.frame-pane {
  display: flex;
  flex-direction: column;
  width: 100%;
  height: 100%;
  overflow: hidden;
  position: relative;
}

.tab-bar {
  display: flex;
  align-items: center;
  background-color: var(--bg-mantle);
  border-bottom: 1px solid #313244;
  flex-shrink: 0;
}

.tabs-scroll {
  display: flex;
  overflow-x: auto;
  min-width: 0;
  flex: 1;
  position: relative; /* tab-drop-indicator の基準 */
}

.tab-button {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 4px 10px;
  font-size: 12px;
  flex-shrink: 0;
  border-right: 1px solid #313244;
  cursor: pointer;
  user-select: none;
  background: var(--bg-mantle);
  border-top: none;
  border-bottom: none;
  border-left: none;
  transition: color 0.15s;
}

.tab-active {
  background-color: var(--bg-base);
  color: #cba6f7;
}

.tab-inactive {
  background-color: var(--bg-mantle);
  color: #6c7086;
}

.tab-inactive:hover {
  color: #cdd6f4;
}

.tab-title {
  max-width: 120px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.tab-close {
  font-size: 10px;
  opacity: 0;
  transition: opacity 0.15s;
  margin-left: 2px;
}

.tab-button:hover .tab-close {
  opacity: 1;
}

.tab-close:hover {
  color: #f38ba8;
}

.add-button {
  padding: 4px 10px;
  font-size: 14px;
  color: #6c7086;
  background: transparent;
  border: none;
  cursor: pointer;
  flex-shrink: 0;
  transition: color 0.15s;
}

.add-button:hover {
  color: #cdd6f4;
}

.terminal-area {
  position: relative;
  flex: 1;
  min-height: 0;
}

.terminal-slot {
  position: absolute;
  inset: 0;
}

.terminal-teleport-target {
  width: 100%;
  height: 100%;
}

.empty-pane {
  display: flex;
  align-items: center;
  justify-content: center;
  height: 100%;
  color: #6c7086;
  font-size: 14px;
}

.tab-drop-indicator {
  position: absolute;
  top: 0;
  bottom: 0;
  width: 2px;
  background-color: #cba6f7;
  border-radius: 1px;
  box-shadow: 0 0 4px rgba(203, 166, 247, 0.6);
  pointer-events: none;
  z-index: 10;
  transition: left 0.05s ease;
}
</style>

<i18n lang="json">
{
  "en": {
    "addTerminal": "Add terminal",
    "noTerminals": "No terminals"
  },
  "ja": {
    "addTerminal": "ターミナルを追加",
    "noTerminals": "ターミナルがありません"
  }
}
</i18n>
