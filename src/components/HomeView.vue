<script setup lang="ts">
import type { Worktree } from "../types/worktree";
import type { TaskItem } from "../types/task";
import { useI18n } from "vue-i18n";

const { t } = useI18n();
import WorktreeCard from "./WorktreeCard.vue";
import TaskCard from "./TaskCard.vue";
import { useMasonryLayout } from "../composables/useMasonryLayout";
import { computed, ref } from "vue";

const draggingId = ref<string | null>(null);
let swapCooldown = false;
let originalOrder: string[] = [];
let dropped = false;

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
    emit("reorderWorktrees", draggingId.value, worktreeId);
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
  loadingWorktrees: Map<string, string>;
  autoApprovals: Map<string, boolean>;
  aiJudgingWorktrees: Set<string>;
  tasks: TaskItem[];
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
  addTask: [];
  removeTask: [taskId: string];
  rerunTask: [taskId: string];
}>();

type PanelMode = "worktree" | "task";
const panelMode = ref<PanelMode>("worktree");

const tasksRef = computed(() => props.tasks);

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

const { containerRef: taskContainerRef, columns: taskColumns } = useMasonryLayout(tasksRef, { minColumnWidth: TASK_CARD_WIDTH, gap: 12 });
</script>

<template>
  <div class="home-view">
    <div class="home-header">
      <select v-model="panelMode" class="panel-select">
        <option value="worktree">{{ t('worktreeTitle') }}</option>
        <option value="task">{{ t('taskTitle') }}</option>
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
        <template v-else>
          <button class="btn-icon-header" :title="t('addTaskButton')" @click="emit('addTask')">
            <i class="pi pi-plus"></i>
          </button>
        </template>
      </div>
    </div>

    <!-- ワークツリーパネル -->
    <template v-if="panelMode === 'worktree'">
      <div v-if="worktrees.length === 0" class="empty-state">
        {{ t('worktreeEmpty') }}
      </div>

      <TransitionGroup
        name="card"
        tag="div"
        class="worktree-grid"
        :style="{ gridTemplateColumns: `repeat(auto-fill, minmax(${naturalCardWidth}px, 1fr))` }"
      >
        <div
          v-for="worktree in worktrees"
          :key="worktree.id"
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
          />
        </div>
      </TransitionGroup>
    </template>

    <!-- タスクパネル -->
    <template v-else>
      <div v-if="tasks.length === 0" class="empty-state">
        {{ t('taskEmpty') }}
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
    </template>
  </div>
</template>

<style scoped>
.home-view {
  width: 100%;
  height: 100%;
  overflow-y: auto;
  padding: 16px;
  background: transparent;
  box-sizing: border-box;
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

.worktree-grid {
  display: grid;
  gap: 12px;
  align-items: start;
  position: relative;
}

/* TransitionGroup: カード移動（FLIP） */
.card-move {
  transition: transform 0.3s ease;
}

/* TransitionGroup: カード削除 */
.card-leave-active {
  transition: opacity 0.25s ease, transform 0.25s ease;
  position: absolute;
  pointer-events: none;
}

.card-leave-to {
  opacity: 0;
  transform: scale(0.85);
}

/* TransitionGroup: カード追加 */
.card-enter-active {
  transition: opacity 0.25s ease, transform 0.25s ease;
}

.card-enter-from {
  opacity: 0;
  transform: scale(0.85);
}

.card-drop-target {
  min-width: 0;
}

/* タスクパネル（既存のマソンリーレイアウト） */
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
    "addTaskButton": "Add task"
  },
  "ja": {
    "worktreeTitle": "ワークツリー",
    "worktreeEmpty": "ワークツリーがありません。右上の + ボタンで作成してください。",
    "focusAllSubWindows": "すべてのサブウィンドウを呼び出す",
    "addWorktreeButton": "ワークツリー追加",
    "taskTitle": "タスク",
    "taskEmpty": "タスクがありません。+ ボタンで追加してください。",
    "addTaskButton": "タスクを追加"
  }
}
</i18n>
