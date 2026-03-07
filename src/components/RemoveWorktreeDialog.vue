<script setup lang="ts">
import { ref, watch } from "vue";

defineProps<{
  worktreeName: string;
  branchName: string;
  branches: string[];
  submitting?: boolean;
}>();

const emit = defineEmits<{
  confirm: [options: { mergeTo: string; deleteBranch: boolean }];
  cancel: [];
}>();

const mergeTo = ref("");
const deleteBranch = ref(false);

watch(mergeTo, (val) => {
  if (!val) deleteBranch.value = false;
});

function confirm() {
  emit("confirm", { mergeTo: mergeTo.value, deleteBranch: deleteBranch.value });
}
</script>

<template>
  <div class="dialog-overlay" @click.self="!submitting && emit('cancel')">
    <div class="dialog">
      <h3 class="dialog-title">ワークツリー「{{ worktreeName }}」を削除</h3>

      <div class="branch-info">
        <span class="label">ブランチ:</span>
        <span class="branch-name">{{ branchName }}</span>
      </div>

      <div class="field">
        <label class="label">マージ先ブランチ（任意）</label>
        <select v-model="mergeTo" class="select" :disabled="submitting">
          <option value="">マージしない</option>
          <option v-for="b in branches" :key="b" :value="b">{{ b }}</option>
        </select>
      </div>

      <div class="field checkbox-field">
        <label :class="['checkbox-label', !mergeTo ? 'disabled' : '']">
          <input
            v-model="deleteBranch"
            type="checkbox"
            :disabled="!mergeTo || submitting"
          />
          ブランチを削除する
        </label>
      </div>

      <p class="warn">⚠ git worktree remove が実行されます</p>

      <div class="dialog-actions">
        <button class="btn-cancel" :disabled="submitting" @click="emit('cancel')">キャンセル</button>
        <button class="btn-danger" :disabled="submitting" @click="confirm">
          <span v-if="submitting" class="pi pi-spinner pi-spin" style="margin-right: 6px;" />
          {{ submitting ? '削除中...' : '削除' }}
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

.checkbox-label.disabled {
  color: #45475a;
  cursor: not-allowed;
}

.checkbox-label input[type="checkbox"] {
  cursor: pointer;
  accent-color: #f38ba8;
}

.checkbox-label.disabled input[type="checkbox"] {
  cursor: not-allowed;
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
