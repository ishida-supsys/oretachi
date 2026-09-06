<script setup lang="ts">
import { useI18n } from "vue-i18n";
import { useWorkgroups } from "../composables/useWorkgroups";
import type { Workgroup } from "../types/settings";

const { t } = useI18n();
const { groups, displayName, worktreeCount } = useWorkgroups();

defineProps<{
  /** 移動対象のワークツリー名（ダイアログ見出しに出す） */
  worktreeName: string;
  /** 現在の所属グループID（フォールバック解決済み） */
  currentGroupId: string;
}>();

const emit = defineEmits<{
  select: [groupId: string];
  cancel: [];
}>();

function onSelect(group: Workgroup, isCurrent: boolean) {
  // 現在のグループを選んでも何も起きないので、そのまま閉じるだけにする
  if (isCurrent) {
    emit("cancel");
    return;
  }
  emit("select", group.id);
}
</script>

<template>
  <div class="dialog-overlay" @click.self="emit('cancel')">
    <div class="dialog">
      <h3 class="dialog-title">{{ t('title') }}</h3>
      <p class="dialog-subtitle">{{ worktreeName }}</p>

      <div class="group-list">
        <button
          v-for="g in groups"
          :key="g.id"
          class="group-item"
          :class="{ current: g.id === currentGroupId }"
          @click="onSelect(g, g.id === currentGroupId)"
        >
          <span class="group-dot" :style="{ background: g.color || '#9399b2' }" />
          <span class="group-name">{{ displayName(g) }}</span>
          <span class="group-count">{{ worktreeCount(g.id) }}</span>
          <span v-if="g.id === currentGroupId" class="pi pi-check group-check" />
        </button>
      </div>

      <p class="hint">{{ t('dragHint') }}</p>

      <div class="dialog-actions">
        <button class="btn-cancel" @click="emit('cancel')">{{ t('common.cancel') }}</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.dialog-overlay {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.6);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 100;
}

.dialog {
  background: #1e1e2e;
  border: 1px solid #313244;
  border-radius: 10px;
  padding: 20px;
  width: 360px;
  max-width: 90vw;
  max-height: 88vh;
  display: flex;
  flex-direction: column;
}

.dialog-title {
  margin: 0;
  font-size: 15px;
  font-weight: 700;
  color: #cdd6f4;
}

.dialog-subtitle {
  margin: 4px 0 14px;
  font-size: 12px;
  color: #9399b2;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* グループが多くてもダイアログ内でスクロールさせ、画面外へはみ出させない */
.group-list {
  display: flex;
  flex-direction: column;
  gap: 4px;
  overflow-y: auto;
  min-height: 0;
}

.group-item {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  padding: 8px 10px;
  border-radius: 7px;
  border: 1px solid #313244;
  background: #181825;
  color: #cdd6f4;
  font-size: 13px;
  cursor: pointer;
  text-align: left;
}

.group-item:hover {
  background: #232336;
  border-color: #585b70;
}

.group-item.current {
  border-color: #585b70;
  background: #232336;
}

.group-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  flex-shrink: 0;
}

.group-name {
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.group-count {
  background: #45475a;
  color: #bac2de;
  border-radius: 8px;
  padding: 0 6px;
  font-size: 10px;
  font-weight: 700;
  min-width: 16px;
  text-align: center;
}

.group-check {
  color: var(--p-green-400);
  font-size: 12px;
}

.hint {
  margin: 12px 0 0;
  font-size: 11px;
  color: #6c7086;
}

.dialog-actions {
  display: flex;
  justify-content: flex-end;
  margin-top: 14px;
}

.btn-cancel {
  padding: 6px 14px;
  border-radius: 6px;
  border: 1px solid #45475a;
  background: transparent;
  color: #a6adc8;
  font-size: 12px;
  cursor: pointer;
}

.btn-cancel:hover {
  background: #313244;
  color: #cdd6f4;
}
</style>

<i18n lang="json">
{
  "en": {
    "title": "Move to group",
    "dragHint": "You can also drag a worktree card onto a group tab."
  },
  "ja": {
    "title": "グループ移動",
    "dragHint": "ワークツリーカードをグループタブへドラッグしても移動できます。"
  }
}
</i18n>
