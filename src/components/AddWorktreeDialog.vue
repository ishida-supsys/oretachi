<script setup lang="ts">
import { ref, computed, watch } from "vue";
import type { Repository, WorktreeEntry } from "../types/settings";

const props = defineProps<{
  repositories: Repository[];
  worktreeBaseDir: string;
  submitting?: boolean;
}>();

const emit = defineEmits<{
  confirm: [entry: WorktreeEntry];
  cancel: [];
}>();

const selectedRepoId = ref<string>("");
const worktreeName = ref("");
const branchName = ref("");
const branchManuallyEdited = ref(false);

const selectedRepo = computed(() =>
  props.repositories.find((r) => r.id === selectedRepoId.value) ?? null
);

watch(worktreeName, (name) => {
  if (!branchManuallyEdited.value) {
    branchName.value = name ? `worktree/${name}` : "";
  }
});

const worktreePath = computed(() => {
  if (!props.worktreeBaseDir || !worktreeName.value) return "";
  return `${props.worktreeBaseDir}/${worktreeName.value}`;
});

function randomSuffix() {
  return Math.random().toString(36).slice(2, 6);
}

function prefill() {
  if (selectedRepo.value && !worktreeName.value) {
    worktreeName.value = `${selectedRepo.value.name}-${randomSuffix()}`;
    branchManuallyEdited.value = false;
  }
}

function confirm() {
  if (!selectedRepo.value || !worktreeName.value) return;

  const entry: WorktreeEntry = {
    id: `${Date.now()}-${randomSuffix()}`,
    name: worktreeName.value,
    repositoryId: selectedRepo.value.id,
    repositoryName: selectedRepo.value.name,
    path: worktreePath.value,
    branchName: branchName.value,
  };

  emit("confirm", entry);
}
</script>

<template>
  <div class="dialog-overlay" @click.self="!submitting && emit('cancel')">
    <div class="dialog">
      <h3 class="dialog-title">ワークツリーを追加</h3>

      <!-- リポジトリ選択 -->
      <div class="field">
        <label class="label">リポジトリ</label>
        <select
          v-model="selectedRepoId"
          class="select"
          :disabled="submitting"
          @change="prefill"
        >
          <option value="">選択してください</option>
          <option
            v-for="repo in repositories"
            :key="repo.id"
            :value="repo.id"
          >
            {{ repo.name }}
          </option>
        </select>
      </div>

      <!-- ワークツリー名 -->
      <div class="field">
        <label class="label">ワークツリー名</label>
        <input
          v-model="worktreeName"
          class="input"
          placeholder="例: my-feature-a3f2"
          :disabled="submitting"
        />
      </div>

      <!-- ブランチ名 -->
      <div class="field">
        <label class="label">ブランチ名</label>
        <input
          v-model="branchName"
          class="input"
          placeholder="例: worktree/my-feature"
          :disabled="submitting"
          @input="branchManuallyEdited = true"
        />
      </div>

      <!-- パス（自動） -->
      <div class="field">
        <label class="label">作成先パス（自動）</label>
        <input class="input readonly" :value="worktreePath" readonly />
      </div>

      <!-- ボタン -->
      <div class="dialog-actions">
        <button class="btn-cancel" :disabled="submitting" @click="emit('cancel')">キャンセル</button>
        <button
          class="btn-confirm"
          :disabled="!selectedRepo || !worktreeName || !worktreeBaseDir || submitting"
          @click="confirm"
        >
          <span v-if="submitting" class="pi pi-spinner pi-spin" style="margin-right: 6px;" />
          {{ submitting ? '作成中...' : '作成' }}
        </button>
      </div>

      <p v-if="!worktreeBaseDir" class="warn">
        設定でワークツリー追加先ディレクトリを設定してください。
      </p>
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
  color: #cba6f7;
  margin: 0 0 20px;
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

.input,
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

.dialog-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  margin-top: 20px;
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

.btn-confirm {
  background: #cba6f7;
  color: #1e1e2e;
  border: none;
  border-radius: 4px;
  padding: 7px 16px;
  font-size: 13px;
  font-weight: 600;
  cursor: pointer;
}

.btn-confirm:hover:not(:disabled) {
  background: #b4befe;
}

.btn-confirm:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}

.warn {
  margin-top: 12px;
  font-size: 12px;
  color: #f9e2af;
}
</style>
