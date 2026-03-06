<script setup lang="ts">
import type { Worktree } from "../types/worktree";
import WorktreeCard from "./WorktreeCard.vue";
import { useMasonryLayout } from "../composables/useMasonryLayout";
import { computed } from "vue";

const props = defineProps<{
  worktrees: Worktree[];
  thumbnailUrls: Map<number, string>;
  detachedWorktrees: Set<string>;
  notifications: Map<string, number>;
  hotkeyChars: Map<string, string>;
  loadingWorktrees: Set<string>;
  autoApprovals: Map<string, boolean>;
  aiJudgingWorktrees: Set<string>;
}>();

const emit = defineEmits<{
  selectTerminal: [terminalId: number];
  addWorktree: [];
  removeWorktree: [worktreeId: string];
  addTerminal: [worktreeId: string];
  openInIde: [worktreeId: string];
  moveToSubWindow: [worktreeId: string];
  moveToMainWindow: [worktreeId: string];
  focusSubWindow: [worktreeId: string];
  setHotkeyChar: [worktreeId: string];
  toggleAutoApproval: [worktreeId: string];
  cancelAiJudging: [worktreeId: string];
}>();

const worktreesRef = computed(() => props.worktrees);

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

const { containerRef, columns } = useMasonryLayout(worktreesRef, { minColumnWidth: naturalCardWidth, gap: 12 });
</script>

<template>
  <div class="home-view">
    <div class="home-header">
      <span class="home-title">ワークツリー一覧</span>
      <button class="btn-add" @click="emit('addWorktree')">+ ワークツリー追加</button>
    </div>

    <div v-if="worktrees.length === 0" class="empty-state">
      ワークツリーがありません。「+ ワークツリー追加」で作成してください。
    </div>

    <div ref="containerRef" class="worktree-list">
      <div v-for="(col, colIndex) in columns" :key="colIndex" class="masonry-column" :style="{ maxWidth: naturalCardWidth + 'px' }">
        <WorktreeCard
          v-for="worktree in col"
          :key="worktree.id"
          :worktree="worktree"
          :thumbnail-urls="thumbnailUrls"
          :detached="detachedWorktrees.has(worktree.id)"
          :notification-count="notifications.get(worktree.id) ?? 0"
          :hotkey-char="hotkeyChars.get(worktree.id)"
          :loading="loadingWorktrees.has(worktree.id)"
          :auto-approval="autoApprovals.get(worktree.id) ?? false"
          :ai-judging="aiJudgingWorktrees.has(worktree.id)"
          @select-terminal="emit('selectTerminal', $event)"
          @add-terminal="emit('addTerminal', $event)"
          @remove-worktree="emit('removeWorktree', $event)"
          @open-in-ide="emit('openInIde', $event)"
          @move-to-sub-window="emit('moveToSubWindow', $event)"
          @move-to-main-window="emit('moveToMainWindow', $event)"
          @focus-sub-window="emit('focusSubWindow', $event)"
          @set-hotkey-char="emit('setHotkeyChar', $event)"
          @toggle-auto-approval="emit('toggleAutoApproval', $event)"
          @cancel-ai-judging="emit('cancelAiJudging', $event)"
        />
      </div>
    </div>
  </div>
</template>

<style scoped>
.home-view {
  width: 100%;
  height: 100%;
  overflow-y: auto;
  padding: 16px;
  background: #1e1e2e;
  box-sizing: border-box;
}

.home-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 16px;
}

.home-title {
  font-size: 15px;
  font-weight: 600;
  color: #cdd6f4;
}

.btn-add {
  background: #cba6f7;
  color: #1e1e2e;
  border: none;
  border-radius: 4px;
  padding: 6px 14px;
  font-size: 12px;
  font-weight: 600;
  cursor: pointer;
}

.btn-add:hover {
  background: #b4befe;
}

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
