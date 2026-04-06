<script setup lang="ts">
import type { Worktree } from "../types/worktree";
import { useI18n } from "vue-i18n";

const { t } = useI18n();
import WorktreeCard from "./WorktreeCard.vue";
import TaskCard from "./TaskCard.vue";
import ArchiveTable from "./ArchiveTable.vue";
import HomeCatTerminal from "./HomeCatTerminal.vue";
import { useMasonryLayout } from "../composables/useMasonryLayout";
import { useSettings } from "../composables/useSettings";
import { useTasks } from "../composables/useTasks";
import { useTaskPersistence } from "../composables/useTaskPersistence";
import { useTaskSearch } from "../composables/useTaskSearch";
import { useInfiniteScroll } from "../composables/useInfiniteScroll";
import { useArchivePersistence, deleteArchive } from "../composables/useArchivePersistence";
import { computed, nextTick, reactive, ref, watch } from "vue";
import autoAnimate from "@formkit/auto-animate";
import type { AnimationController } from "@formkit/auto-animate";

const draggingId = ref<string | null>(null);
let swapCooldown = false;
let originalOrder: string[] = [];
let dropped = false;

// カスタムFLIPアニメーション
const cardElements = new Map<string, HTMLElement>();

function registerCard(id: string, el: HTMLElement | null) {
  if (el) cardElements.set(id, el);
  else cardElements.delete(id);
}

function capturePositions(): Map<string, DOMRect> {
  const positions = new Map<string, DOMRect>();
  for (const [id, el] of cardElements) {
    positions.set(id, el.getBoundingClientRect());
  }
  return positions;
}

function animateFlip(oldPositions: Map<string, DOMRect>) {
  for (const [id, el] of cardElements) {
    const oldRect = oldPositions.get(id);
    if (!oldRect) continue;
    const newRect = el.getBoundingClientRect();
    const dx = oldRect.left - newRect.left;
    const dy = oldRect.top - newRect.top;
    if (dx === 0 && dy === 0) continue;

    el.style.transform = `translate(${dx}px, ${dy}px)`;
    el.style.transition = '';
    // Force reflow
    el.offsetHeight;
    el.style.transition = 'transform 0.3s ease';
    el.style.transform = '';
  }
}

// auto-animate: 各カラムのコントローラーを管理
const columnControllers = new Map<number, AnimationController>();

function registerColumn(el: HTMLElement | null, index: number) {
  if (!el) {
    columnControllers.delete(index);
    return;
  }
  if (!columnControllers.has(index)) {
    columnControllers.set(index, autoAnimate(el, { duration: 250 }));
  }
}

// 削除アニメーション用
const hiddenWorktrees = reactive(new Set<string>());
let autoAnimateDisableDepth = 0;

async function fadeOutCard(worktreeId: string): Promise<void> {
  const el = cardElements.get(worktreeId);
  if (!el) return;
  const anim = el.animate([{ opacity: 1 }, { opacity: 0 }], {
    duration: 250,
    easing: "ease-out",
    fill: "forwards",
  });
  await anim.finished;
}

function hideCard(worktreeId: string): Map<string, DOMRect> {
  hiddenWorktrees.add(worktreeId);
  // splice 前に残りカードの位置を記録し、autoAnimate を無効化（FLIP に委ねる）
  const positions = capturePositions();
  if (++autoAnimateDisableDepth === 1) {
    for (const ctrl of columnControllers.values()) ctrl.disable();
  }
  return positions;
}

function animateAfterRemove(positions: Map<string, DOMRect>): void {
  animateFlip(positions);
  if (--autoAnimateDisableDepth <= 0) {
    autoAnimateDisableDepth = 0;
    for (const ctrl of columnControllers.values()) ctrl.enable();
  }
}

function unhideCard(worktreeId: string): void {
  hiddenWorktrees.delete(worktreeId);
}

const { settings } = useSettings();
const catRef = ref<InstanceType<typeof HomeCatTerminal> | null>(null);

function sendCatTopic(text: string, priority: number) {
  catRef.value?.topic(text, priority);
}

defineExpose({ fadeOutCard, hideCard, animateAfterRemove, unhideCard, sendCatTopic });

// D&D中はauto-animateを無効化してカスタムFLIPに委ねる
watch(draggingId, (id) => {
  for (const ctrl of columnControllers.values()) {
    if (id) ctrl.disable();
    else ctrl.enable();
  }
});

function onCardDragStart(worktreeId: string, event: DragEvent) {
  draggingId.value = worktreeId;
  originalOrder = props.worktrees.map((w) => w.id);
  dropped = false;
  event.dataTransfer?.setData("text/plain", worktreeId);
}

function onCardDragOver(worktreeId: string, event: DragEvent) {
  event.preventDefault();
  if (swapCooldown) return;
  if (draggingId.value && draggingId.value !== worktreeId) {
    const oldPositions = capturePositions();
    emit("reorderWorktrees", draggingId.value, worktreeId);
    nextTick(() => animateFlip(oldPositions));
    swapCooldown = true;
    setTimeout(() => { swapCooldown = false; }, 300);
  }
}

function onCardDrop(event: DragEvent) {
  event.preventDefault();
  dropped = true;
  draggingId.value = null;
  emit("commitReorder");
}

function onDragEnd() {
  if (!dropped) {
    emit("cancelReorder", originalOrder);
  }
  draggingId.value = null;
  dropped = false;
}

const props = defineProps<{
  worktrees: Worktree[];
  thumbnailUrls: Map<number, string>;
  detachedWorktrees: Set<string>;
  notifications: Map<string, number>;
  hotkeyChars: Map<string, string>;
  artifactCounts: Map<string, number>;
  loadingWorktrees: Map<string, string>;
  autoApprovals: Map<string, boolean>;
  aiJudgingWorktrees: Set<string>;
}>();

const emit = defineEmits<{
  selectTerminal: [terminalId: number];
  reorderWorktrees: [fromId: string, toId: string];
  commitReorder: [];
  cancelReorder: [orderIds: string[]];
  addWorktree: [];
  removeWorktree: [worktreeId: string];
  addTerminal: [worktreeId: string];
  openInIde: [worktreeId: string];
  openArtifacts: [worktreeId: string];
  moveToSubWindow: [worktreeId: string];
  moveToMainWindow: [worktreeId: string];
  focusSubWindow: [worktreeId: string];
  focusAllSubWindows: [];
  setHotkeyChar: [worktreeId: string];
  toggleAutoApproval: [worktreeId: string];
  cancelAiJudging: [worktreeId: string];
  duplicateWorktree: [worktreeId: string];
  addTask: [];
  removeTask: [taskId: string];
  rerunTask: [taskId: string];
}>();

type PanelMode = "worktree" | "task" | "archive";
const panelMode = ref<PanelMode>("worktree");

const { sortedTasks } = useTasks();
const { hasMore, isLoading, loadTasks, loadMore, search } = useTaskPersistence();
const {
  archives,
  hasMore: archiveHasMore,
  isLoading: archiveIsLoading,
  loadArchives,
  loadMore: archiveLoadMore,
  searchArchives,
} = useArchivePersistence();

const { taskSearchInput, onSearchInput, clearSearch } = useTaskSearch(search);

const archiveSearchInput = ref("");

async function onArchiveSearchInput() {
  await searchArchives(archiveSearchInput.value);
}

function clearArchiveSearch() {
  archiveSearchInput.value = "";
  searchArchives("");
}

const scrollSentinelRef = ref<HTMLElement | null>(null);
const { setup: setupScroll, teardown: teardownScroll } = useInfiniteScroll(
  scrollSentinelRef,
  loadMore,
  { hasMore, isLoading }
);

const archiveScrollSentinelRef = ref<HTMLElement | null>(null);
const { setup: setupArchiveScroll, teardown: teardownArchiveScroll } = useInfiniteScroll(
  archiveScrollSentinelRef,
  archiveLoadMore,
  { hasMore: archiveHasMore, isLoading: archiveIsLoading }
);

watch(panelMode, async (mode, oldMode) => {
  if (oldMode === "task") teardownScroll();
  if (oldMode === "archive") teardownArchiveScroll();

  if (mode === "task") {
    await loadTasks(true);
    nextTick(() => setupScroll());
  } else if (mode === "archive") {
    await loadArchives(true);
    nextTick(() => setupArchiveScroll());
  }
});

const worktreesRef = computed(() => props.worktrees);
const tasksRef = sortedTasks;

// 各ワークツリーカードの自然幅（ターミナルサムネイル幅から計算）をもとに列幅を決定する
const naturalCardWidth = computed(() => {
  const THUMBNAIL_W = 107;  // TerminalThumbnail の固定幅
  const THUMBNAIL_GAP = 8;  // .terminals-row の gap
  const CARD_PADDING = 24;  // .worktree-card の padding 12px × 2
  const MIN_WIDTH = 260;    // ヘッダーボタンが収まる最小幅

  let max = MIN_WIDTH;
  for (const wt of props.worktrees) {
    const n = wt.terminals.length;
    if (n <= 0) continue;
    const w = CARD_PADDING + n * THUMBNAIL_W + (n - 1) * THUMBNAIL_GAP;
    if (w > max) max = w;
  }
  return max;
});

const TASK_CARD_WIDTH = ref(260);

const { containerRef, columns } = useMasonryLayout(worktreesRef, { minColumnWidth: naturalCardWidth, gap: 12 });
const { containerRef: taskContainerRef, columns: taskColumns } = useMasonryLayout(tasksRef, { minColumnWidth: TASK_CARD_WIDTH, gap: 12 });

</script>

<template>
  <div class="home-view">
    <!-- 背景: ターミナル猫 -->
    <div v-if="settings.enableHomeCat === true" class="home-cat-bg">
      <HomeCatTerminal ref="catRef" />
    </div>

    <!-- フォアグラウンドコンテンツ -->
    <div class="home-content">
    <div class="home-header">
      <select v-model="panelMode" class="panel-select">
        <option value="worktree">{{ t('worktreeTitle') }}</option>
        <option value="task">{{ t('taskTitle') }}</option>
        <option value="archive">{{ t('archiveTitle') }}</option>
      </select>
      <div class="header-actions">
        <template v-if="panelMode === 'worktree'">
          <button class="btn-icon-header" :title="t('focusAllSubWindows')" @click="emit('focusAllSubWindows')">
            <i class="pi pi-window-maximize"></i>
          </button>
          <button class="btn-icon-header" :title="t('addWorktreeButton')" @click="emit('addWorktree')">
            <i class="pi pi-plus"></i>
          </button>
        </template>
        <template v-else-if="panelMode === 'task'">
          <div class="task-search">
            <i class="pi pi-search search-icon" />
            <input
              v-model="taskSearchInput"
              type="text"
              class="task-search-input"
              :placeholder="t('taskSearchPlaceholder')"
              @input="onSearchInput"
            />
            <button v-if="taskSearchInput" class="btn-clear-search" @click="clearSearch">
              <i class="pi pi-times" />
            </button>
          </div>
          <button class="btn-icon-header" :title="t('addTaskButton')" @click="emit('addTask')">
            <i class="pi pi-plus"></i>
          </button>
        </template>
        <template v-else>
          <div class="task-search">
            <i class="pi pi-search search-icon" />
            <input
              v-model="archiveSearchInput"
              type="text"
              class="task-search-input"
              :placeholder="t('archiveSearchPlaceholder')"
              @input="onArchiveSearchInput"
            />
            <button v-if="archiveSearchInput" class="btn-clear-search" @click="clearArchiveSearch">
              <i class="pi pi-times" />
            </button>
          </div>
        </template>
      </div>
    </div>

    <!-- ワークツリーパネル -->
    <template v-if="panelMode === 'worktree'">
      <div v-if="worktrees.length === 0" class="empty-state">
        {{ t('worktreeEmpty') }}
      </div>

      <div ref="containerRef" class="worktree-list">
        <div
          v-for="(col, colIndex) in columns"
          :key="colIndex"
          :ref="(el) => registerColumn(el as HTMLElement, colIndex)"
          class="masonry-column"
          :style="{ maxWidth: naturalCardWidth + 'px' }"
        >
          <div
            v-for="worktree in col"
            :key="worktree.id"
            :ref="(el) => registerCard(worktree.id, el as HTMLElement)"
            v-show="!hiddenWorktrees.has(worktree.id)"
            class="card-drop-target"
            @dragover="onCardDragOver(worktree.id, $event)"
            @drop="onCardDrop"
          >
            <WorktreeCard
              :worktree="worktree"
              :thumbnail-urls="thumbnailUrls"
              :detached="detachedWorktrees.has(worktree.id)"
              :notification-count="notifications.get(worktree.id) ?? 0"
              :hotkey-char="hotkeyChars.get(worktree.id)"
              :artifact-count="artifactCounts.get(worktree.id) ?? 0"
              :loading="loadingWorktrees.has(worktree.id)"
              :loading-text="loadingWorktrees.get(worktree.id)"
              :auto-approval="autoApprovals.get(worktree.id) ?? false"
              :ai-judging="aiJudgingWorktrees.has(worktree.id)"
              @drag-start="onCardDragStart"
              @drag-end="onDragEnd"
              @select-terminal="emit('selectTerminal', $event)"
              @add-terminal="emit('addTerminal', $event)"
              @remove-worktree="emit('removeWorktree', $event)"
              @open-in-ide="emit('openInIde', $event)"
              @open-artifacts="emit('openArtifacts', $event)"
              @move-to-sub-window="emit('moveToSubWindow', $event)"
              @move-to-main-window="emit('moveToMainWindow', $event)"
              @focus-sub-window="emit('focusSubWindow', $event)"
              @set-hotkey-char="emit('setHotkeyChar', $event)"
              @toggle-auto-approval="emit('toggleAutoApproval', $event)"
              @cancel-ai-judging="emit('cancelAiJudging', $event)"
              @duplicate-worktree="emit('duplicateWorktree', $event)"
            />
          </div>
        </div>
      </div>
    </template>

    <!-- タスクパネル -->
    <template v-else-if="panelMode === 'task'">
      <div v-if="sortedTasks.length === 0 && !isLoading" class="empty-state">
        {{ taskSearchInput ? t('taskNoResults') : t('taskEmpty') }}
      </div>

      <div ref="taskContainerRef" class="worktree-list">
        <div v-for="(col, colIndex) in taskColumns" :key="colIndex" class="masonry-column" :style="{ maxWidth: TASK_CARD_WIDTH + 'px' }">
          <TaskCard
            v-for="task in col"
            :key="task.id"
            :task="task"
            @remove="emit('removeTask', $event)"
            @rerun="emit('rerunTask', $event)"
          />
        </div>
      </div>

      <!-- 無限スクロール センチネル -->
      <div ref="scrollSentinelRef" class="scroll-sentinel">
        <i v-if="isLoading" class="pi pi-spinner pi-spin loading-spinner" />
      </div>
    </template>

    <!-- アーカイブパネル -->
    <template v-else>
      <div v-if="archives.length === 0 && !archiveIsLoading" class="empty-state">
        {{ t('archiveEmpty') }}
      </div>

      <ArchiveTable :items="archives" @delete="deleteArchive" />

      <!-- 無限スクロール センチネル -->
      <div ref="archiveScrollSentinelRef" class="scroll-sentinel">
        <i v-if="archiveIsLoading" class="pi pi-spinner pi-spin loading-spinner" />
      </div>
    </template>
    </div><!-- /home-content -->
  </div>
</template>

<style scoped>
.home-view {
  width: 100%;
  height: 100%;
  background: transparent;
  position: relative;
}

.home-cat-bg {
  position: absolute;
  inset: 0;
  z-index: 0;
  pointer-events: none;
}

.home-content {
  position: relative;
  z-index: 1;
  width: 100%;
  height: 100%;
  overflow-y: auto;
  padding: 16px;
  box-sizing: border-box;
  background: transparent;
}

.home-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 16px;
}

.panel-select {
  font-size: 15px;
  font-weight: 600;
  color: #cdd6f4;
  background: transparent;
  border: none;
  outline: none;
  cursor: pointer;
  padding: 2px 4px;
  border-radius: 4px;
  appearance: none;
  -webkit-appearance: none;
  background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='10' height='6' viewBox='0 0 10 6'%3E%3Cpath d='M0 0l5 6 5-6z' fill='%236c7086'/%3E%3C/svg%3E");
  background-repeat: no-repeat;
  background-position: right 4px center;
  padding-right: 20px;
  transition: color 0.15s;
}

.panel-select:hover {
  color: #cba6f7;
}

.panel-select option {
  background: #1e1e2e;
  color: #cdd6f4;
  font-weight: 600;
}

.header-actions {
  display: flex;
  align-items: center;
  gap: 4px;
}

.btn-icon-header {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 28px;
  height: 28px;
  background: transparent;
  color: #6c7086;
  border: none;
  border-radius: 4px;
  font-size: 14px;
  cursor: pointer;
  transition: color 0.15s, background 0.15s;
}

.btn-icon-header:hover {
  color: #cba6f7;
  background: #313244;
}

/* ワークツリーパネル（マソンリーレイアウト） */
.worktree-list {
  display: flex;
  flex-wrap: wrap;
  gap: 12px;
  align-items: flex-start;
}

.masonry-column {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.card-drop-target {
  border-radius: 8px;
  transition: outline 0.1s;
}

/* タスク検索バー */
.task-search {
  display: flex;
  align-items: center;
  gap: 4px;
  background: #1e1e2e;
  border: 1px solid #313244;
  border-radius: 6px;
  padding: 3px 8px;
  flex: 1;
  max-width: 280px;
  margin-right: 4px;
}

.task-search-input {
  background: transparent;
  border: none;
  outline: none;
  color: #cdd6f4;
  font-size: 13px;
  flex: 1;
  min-width: 0;
}

.search-icon {
  color: #6c7086;
  font-size: 12px;
  flex-shrink: 0;
}

.btn-clear-search {
  background: transparent;
  border: none;
  color: #6c7086;
  cursor: pointer;
  padding: 0;
  font-size: 11px;
  display: flex;
  align-items: center;
  flex-shrink: 0;
}

.btn-clear-search:hover {
  color: #cdd6f4;
}

/* 無限スクロール センチネル */
.scroll-sentinel {
  height: 40px;
  display: flex;
  align-items: center;
  justify-content: center;
}

.loading-spinner {
  color: #6c7086;
  font-size: 16px;
}

.empty-state {
  display: flex;
  align-items: center;
  justify-content: center;
  height: calc(100% - 48px);
  color: #6c7086;
  font-size: 14px;
}
</style>

<i18n lang="json">
{
  "en": {
    "worktreeTitle": "Worktrees",
    "worktreeEmpty": "No worktrees. Click the + button to create one.",
    "focusAllSubWindows": "Bring all sub windows",
    "addWorktreeButton": "Add worktree",
    "taskTitle": "Tasks",
    "taskEmpty": "No tasks. Click the + button to add one.",
    "taskNoResults": "No tasks found.",
    "taskSearchPlaceholder": "Search tasks...",
    "addTaskButton": "Add task",
    "archiveTitle": "Archives",
    "archiveEmpty": "No archives.",
    "archiveSearchPlaceholder": "Search archives..."
  },
  "ja": {
    "worktreeTitle": "ワークツリー",
    "worktreeEmpty": "ワークツリーがありません。右上の + ボタンで作成してください。",
    "focusAllSubWindows": "すべてのサブウィンドウを呼び出す",
    "addWorktreeButton": "ワークツリー追加",
    "taskTitle": "タスク",
    "taskEmpty": "タスクがありません。+ ボタンで追加してください。",
    "taskNoResults": "タスクが見つかりません。",
    "taskSearchPlaceholder": "タスクを検索...",
    "addTaskButton": "タスクを追加",
    "archiveTitle": "アーカイブ",
    "archiveEmpty": "アーカイブがありません。",
    "archiveSearchPlaceholder": "アーカイブを検索..."
  }
}
</i18n>
