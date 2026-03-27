<script setup lang="ts">
import { ref, computed } from "vue";
import { useI18n } from "vue-i18n";

const { t } = useI18n();

const props = defineProps<{
  worktreeName: string;
  branchName: string;
  branches: string[];
  dirtyFiles: { path: string; status: string; staged: boolean }[];
}>();

const dirtyFilesText = computed(() =>
  props.dirtyFiles
    .map((f) => {
      const prefix = f.staged ? "S" : f.status === "??" ? "?" : "M";
      return `${prefix} ${f.path}`;
    })
    .join("\n")
);

const emit = defineEmits<{
  confirm: [options: { mergeTo: string; deleteBranch: boolean; forceBranch: boolean }];
  archive: [options: { mergeTo: string; deleteBranch: boolean; forceBranch: boolean }];
  cancel: [];
}>();

const mergeTo = ref("");
const deleteBranch = ref(false);

const forceBranch = computed(() => deleteBranch.value && !mergeTo.value);

function confirm() {
  emit("confirm", {
    mergeTo: mergeTo.value,
    deleteBranch: deleteBranch.value,
    forceBranch: forceBranch.value,
  });
}

function archive() {
  emit("archive", {
    mergeTo: mergeTo.value,
    deleteBranch: deleteBranch.value,
    forceBranch: forceBranch.value,
  });
}
</script>

<template>
  <div class="dialog-overlay" @click.self="emit('cancel')">
    <div class="dialog">
      <h3 class="dialog-title">{{ t('removeTitle', { name: worktreeName }) }}</h3>

      <div class="branch-info">
        <span class="label">{{ t('branchLabel') }}:</span>
        <span class="branch-name">{{ branchName }}</span>
      </div>

      <div class="field">
        <label class="label">{{ t('mergeTo') }}</label>
        <select v-model="mergeTo" class="select">
          <option value="">{{ t('noMerge') }}</option>
          <option v-for="b in branches" :key="b" :value="b">{{ b }}</option>
        </select>
      </div>

      <div class="field checkbox-field">
        <label class="checkbox-label">
          <input
            v-model="deleteBranch"
            type="checkbox"
          />
          {{ t('deleteBranch') }}
        </label>
      </div>

      <div v-if="props.dirtyFiles.length > 0" class="dirty-files">
        <p class="dirty-files-label">{{ t('dirtyFilesWarning', { count: props.dirtyFiles.length }) }}</p>
        <textarea class="dirty-files-area" readonly :value="dirtyFilesText" />
      </div>

      <p class="warn">{{ t('removeWarning') }}</p>
      <p v-if="forceBranch" class="warn warn-force">{{ t('forceDeleteWarning') }}</p>

      <div class="dialog-actions">
        <button class="btn-cancel" @click="emit('cancel')">{{ t('common.cancel') }}</button>
        <button class="btn-archive" @click="archive">{{ t('archiveButton') }}</button>
        <button class="btn-danger" @click="confirm">{{ t('common.delete') }}</button>
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
  padding: 24px;
  width: 420px;
  max-width: 90vw;
}

.dialog-title {
  font-size: 16px;
  font-weight: 600;
  color: #f38ba8;
  margin: 0 0 16px;
}

.branch-info {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 20px;
  font-size: 13px;
}

.branch-name {
  color: #cdd6f4;
  font-family: monospace;
  background: #313244;
  padding: 2px 8px;
  border-radius: 4px;
}

.field {
  margin-bottom: 14px;
}

.label {
  display: block;
  font-size: 12px;
  color: #a6adc8;
  margin-bottom: 5px;
}

.select {
  width: 100%;
  background: #313244;
  border: 1px solid #45475a;
  border-radius: 4px;
  padding: 7px 10px;
  font-size: 13px;
  color: #cdd6f4;
  outline: none;
  box-sizing: border-box;
}

.select option {
  background: #313244;
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
  accent-color: #f38ba8;
}

.dirty-files {
  margin-bottom: 14px;
}

.dirty-files-label {
  font-size: 12px;
  color: #fab387;
  margin: 0 0 6px;
}

.dirty-files-area {
  width: 100%;
  max-height: 140px;
  background: #181825;
  border: 1px solid #45475a;
  border-radius: 4px;
  padding: 8px 10px;
  font-size: 12px;
  font-family: monospace;
  color: #cdd6f4;
  resize: vertical;
  box-sizing: border-box;
  overflow-y: auto;
}

.warn-force {
  color: #f38ba8;
}

.warn {
  font-size: 12px;
  color: #f9e2af;
  margin: 0 0 20px;
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

.btn-cancel:hover:not(:disabled) {
  background: #45475a;
}

.btn-cancel:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}

.btn-archive {
  background: #313244;
  color: #89b4fa;
  border: 1px solid #45475a;
  border-radius: 4px;
  padding: 7px 16px;
  font-size: 13px;
  font-weight: 600;
  cursor: pointer;
}

.btn-archive:hover:not(:disabled) {
  background: #45475a;
}

.btn-archive:disabled {
  opacity: 0.4;
  cursor: not-allowed;
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

.btn-danger:hover:not(:disabled) {
  background: #eb6f8e;
}

.btn-danger:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}
</style>

<i18n lang="json">
{
  "en": {
    "removeTitle": "Remove worktree \"{name}\"",
    "branchLabel": "Branch",
    "mergeTo": "Merge to branch (optional)",
    "noMerge": "Do not merge",
    "deleteBranch": "Delete branch",
    "removeWarning": "⚠ git worktree remove will be executed",
    "forceDeleteWarning": "⚠ No merge target: git branch -D will force-delete",
    "dirtyFilesWarning": "⚠ {count} uncommitted file(s) will be lost",
    "archiveButton": "Archive"
  },
  "ja": {
    "removeTitle": "ワークツリー「{name}」を削除",
    "branchLabel": "ブランチ",
    "mergeTo": "マージ先ブランチ（任意）",
    "noMerge": "マージしない",
    "deleteBranch": "ブランチを削除する",
    "removeWarning": "⚠ git worktree remove が実行されます",
    "forceDeleteWarning": "⚠ マージ先未指定のため git branch -D で強制削除されます",
    "dirtyFilesWarning": "⚠ {count} 件の未コミットファイルが失われます",
    "archiveButton": "アーカイブ"
  }
}
</i18n>
