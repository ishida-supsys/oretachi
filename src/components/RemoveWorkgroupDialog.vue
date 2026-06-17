<script setup lang="ts">
import { ref } from "vue";
import { useI18n } from "vue-i18n";
import type { Worktree } from "../types/worktree";

const { t } = useI18n();

defineProps<{
  groupName: string;
  members: Worktree[];
}>();

const emit = defineEmits<{
  confirm: [archive: boolean];
  cancel: [];
}>();

const archive = ref(true);
</script>

<template>
  <div class="dialog-overlay" @click.self="emit('cancel')">
    <div class="dialog">
      <h3 class="dialog-title">{{ t('title', { name: groupName }) }}</h3>

      <p class="warn">{{ t('warning', { count: members.length }) }}</p>

      <div v-if="members.length > 0" class="member-list">
        <div class="member-list-label">{{ t('membersLabel') }}</div>
        <div v-for="m in members" :key="m.id" class="member-row">
          <span class="member-name">{{ m.name }}</span>
          <span class="member-branch">{{ m.branchName }}</span>
        </div>
      </div>

      <div class="field checkbox-field">
        <label class="checkbox-label">
          <input v-model="archive" type="checkbox" />
          {{ t('archiveOption') }}
        </label>
      </div>

      <div class="dialog-actions">
        <button class="btn-cancel" @click="emit('cancel')">{{ t('common.cancel') }}</button>
        <button class="btn-danger" @click="emit('confirm', archive)">
          {{ members.length > 0 ? t('confirmWithCount', { count: members.length }) : t('confirmEmpty') }}
        </button>
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
  z-index: 101;
}

.dialog {
  background: #1e1e2e;
  border: 1px solid #313244;
  border-radius: 10px;
  padding: 24px;
  width: 440px;
  max-width: 90vw;
  max-height: 88vh;
  overflow-y: auto;
}

.dialog-title {
  font-size: 16px;
  font-weight: 600;
  color: #f38ba8;
  margin: 0 0 16px;
}

.warn {
  font-size: 13px;
  color: #f9e2af;
  line-height: 1.6;
  margin: 0 0 16px;
}

.member-list {
  background: #181825;
  border: 1px solid #313244;
  border-radius: 6px;
  padding: 10px 12px;
  margin-bottom: 16px;
  max-height: 200px;
  overflow-y: auto;
}

.member-list-label {
  font-size: 11px;
  color: #6c7086;
  margin-bottom: 8px;
}

.member-row {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 12px;
  padding: 3px 0;
}

.member-name {
  font-size: 13px;
  color: #cdd6f4;
  font-weight: 600;
}

.member-branch {
  font-size: 11px;
  color: #6c7086;
  font-family: monospace;
  word-break: break-all;
}

.checkbox-field {
  margin-bottom: 20px;
}

.checkbox-label {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 13px;
  color: #cdd6f4;
  cursor: pointer;
  user-select: none;
}

.checkbox-label input[type="checkbox"] {
  cursor: pointer;
  accent-color: #89b4fa;
}

.dialog-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}

.btn-cancel {
  background: #313244;
  color: #cdd6f4;
  border: 1px solid #45475a;
  border-radius: 4px;
  padding: 7px 16px;
  font-size: 13px;
  cursor: pointer;
}

.btn-cancel:hover {
  background: #45475a;
}

.btn-danger {
  background: #f38ba8;
  color: #1e1e2e;
  border: none;
  border-radius: 4px;
  padding: 7px 16px;
  font-size: 13px;
  font-weight: 600;
  cursor: pointer;
}

.btn-danger:hover {
  background: #eb6f8e;
}
</style>

<i18n lang="json">
{
  "en": {
    "title": "Delete group \"{name}\"",
    "warning": "All {count} worktree(s) in this group will be deleted, then the group will be removed. This cannot be undone.",
    "membersLabel": "Worktrees to be deleted",
    "archiveOption": "Archive before deleting (records the group for restore)",
    "confirmWithCount": "Delete {count} worktree(s) and the group",
    "confirmEmpty": "Delete group"
  },
  "ja": {
    "title": "グループ「{name}」を削除",
    "warning": "所属する {count} 件のワークツリーをすべて削除してから、グループを削除します。この操作は取り消せません。",
    "membersLabel": "削除されるワークツリー",
    "archiveOption": "削除前にアーカイブへ保存する（所属グループも記録）",
    "confirmWithCount": "{count} 件を削除してグループを削除",
    "confirmEmpty": "グループを削除"
  }
}
</i18n>
