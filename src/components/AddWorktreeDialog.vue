<script setup lang="ts">
import { ref, computed, watch, onMounted } from "vue";
import type { Repository, WorktreeEntry } from "../types/settings";
import type { ArchiveRow } from "../types/archive";
import { useI18n } from "vue-i18n";

const { t } = useI18n();

const props = defineProps<{
  repositories: Repository[];
  worktreeBaseDir: string;
  submitting?: boolean;
  activeWorktrees?: WorktreeEntry[];
  archivedWorktrees?: ArchiveRow[];
  duplicateSource?: {
    repositoryId: string;
    worktreeName: string;
    sourceBranch: string;
    sessionSourcePath: string;
  } | null;
}>();

const emit = defineEmits<{
  confirm: [entry: WorktreeEntry, sourceBranch: string | undefined, sessionSourcePath: string | undefined, copyWorkingChangesFrom: string | undefined];
  cancel: [];
}>();

const selectedRepoId = ref<string>("");
const worktreeName = ref("");
const branchName = ref("");
const branchManuallyEdited = ref(false);
const sourceBranch = ref("");
const selectedSessionSource = ref<string>("");
const copyWorkingChanges = ref(true);

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

onMounted(() => {
  if (props.duplicateSource) {
    selectedRepoId.value = props.duplicateSource.repositoryId;
    worktreeName.value = props.duplicateSource.worktreeName;
    branchManuallyEdited.value = false;
    sourceBranch.value = props.duplicateSource.sourceBranch;
    selectedSessionSource.value = props.duplicateSource.sessionSourcePath;
    copyWorkingChanges.value = true;
  }
});

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

  const copyFrom = props.duplicateSource && copyWorkingChanges.value
    ? props.duplicateSource.sessionSourcePath
    : undefined;

  emit("confirm", entry, sourceBranch.value.trim() || undefined, selectedSessionSource.value || undefined, copyFrom);
}
</script>

<template>
  <div class="dialog-overlay" @click.self="!submitting && emit('cancel')">
    <div class="dialog">
      <h3 class="dialog-title">{{ duplicateSource ? t('duplicateTitle') : t('addTitle') }}</h3>

      <!-- リポジトリ選択 -->
      <div class="field">
        <label class="label">{{ t('repo') }}</label>
        <select
          v-model="selectedRepoId"
          class="select"
          :disabled="submitting || !!duplicateSource"
          @change="prefill"
        >
          <option value="">{{ t('repoPlaceholder') }}</option>
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
        <label class="label">{{ t('name') }}</label>
        <input
          v-model="worktreeName"
          class="input"
          :placeholder="t('namePlaceholder')"
          :disabled="submitting"
        />
      </div>

      <!-- ブランチ名 -->
      <div class="field">
        <label class="label">{{ t('branch') }}</label>
        <input
          v-model="branchName"
          class="input"
          :placeholder="t('branchPlaceholder')"
          :disabled="submitting"
          @input="branchManuallyEdited = true"
        />
      </div>

      <!-- 元ブランチ（オプション） -->
      <div class="field">
        <label class="label">{{ t('sourceBranch') }}</label>
        <input
          v-model="sourceBranch"
          class="input"
          :placeholder="t('sourceBranchPlaceholder')"
          :disabled="submitting"
        />
      </div>

      <!-- セッション引継ぎ元（複製時のみ） -->
      <div v-if="duplicateSource" class="field">
        <label class="label">{{ t('sessionSource') }}</label>
        <select v-model="selectedSessionSource" class="select" :disabled="submitting">
          <option value="">{{ t('sessionSourceNone') }}</option>
          <optgroup v-if="activeWorktrees?.length" :label="t('sessionSourceActive')">
            <option v-for="wt in activeWorktrees" :key="wt.path" :value="wt.path">
              {{ wt.name }}
            </option>
          </optgroup>
          <optgroup v-if="archivedWorktrees?.length" :label="t('sessionSourceArchived')">
            <option v-for="ar in archivedWorktrees" :key="ar.path" :value="ar.path">
              {{ ar.name }}
            </option>
          </optgroup>
        </select>
      </div>

      <!-- 未コミット変更コピー（複製時のみ） -->
      <div v-if="duplicateSource" class="field checkbox-field">
        <label class="checkbox-label">
          <input type="checkbox" v-model="copyWorkingChanges" :disabled="submitting" />
          {{ t('copyWorkingChanges') }}
        </label>
      </div>

      <!-- パス（自動） -->
      <div class="field">
        <label class="label">{{ t('path') }}</label>
        <input class="input readonly" :value="worktreePath" readonly />
      </div>

      <!-- ボタン -->
      <div class="dialog-actions">
        <button class="btn-cancel" :disabled="submitting" @click="emit('cancel')">{{ t('common.cancel') }}</button>
        <button
          class="btn-confirm"
          :disabled="!selectedRepo || !worktreeName || !worktreeBaseDir || submitting"
          @click="confirm"
        >
          <span v-if="submitting" class="pi pi-spinner pi-spin" style="margin-right: 6px;" />
          {{ submitting ? (duplicateSource ? t('duplicating') : t('creating')) : (duplicateSource ? t('duplicate') : t('create')) }}
        </button>
      </div>

      <p v-if="!worktreeBaseDir" class="warn">
        {{ t('baseDirNotSet') }}
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

.checkbox-field {
  margin-bottom: 14px;
}

.checkbox-label {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 13px;
  color: #cdd6f4;
  cursor: pointer;
}

.checkbox-label input[type="checkbox"] {
  accent-color: #cba6f7;
  width: 14px;
  height: 14px;
  cursor: pointer;
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

<i18n lang="json">
{
  "en": {
    "addTitle": "Add Worktree",
    "duplicateTitle": "Duplicate Worktree",
    "repo": "Repository",
    "repoPlaceholder": "Select",
    "name": "Worktree name",
    "namePlaceholder": "e.g. my-feature-a3f2",
    "branch": "Branch name",
    "branchPlaceholder": "e.g. worktree/my-feature",
    "path": "Path (auto)",
    "sourceBranch": "Source branch",
    "sourceBranchPlaceholder": "e.g. main, origin/develop (optional)",
    "copyWorkingChanges": "Copy uncommitted changes",
    "creating": "Creating...",
    "create": "Create",
    "duplicating": "Duplicating...",
    "duplicate": "Duplicate",
    "baseDirNotSet": "Set the worktree base directory in settings.",
    "sessionSource": "Inherit session from",
    "sessionSourceNone": "None",
    "sessionSourceActive": "Active worktrees",
    "sessionSourceArchived": "Archived worktrees"
  },
  "ja": {
    "addTitle": "ワークツリーを追加",
    "duplicateTitle": "ワークツリーを複製",
    "repo": "リポジトリ",
    "repoPlaceholder": "選択してください",
    "name": "ワークツリー名",
    "namePlaceholder": "例: my-feature-a3f2",
    "branch": "ブランチ名",
    "branchPlaceholder": "例: worktree/my-feature",
    "sourceBranch": "元ブランチ",
    "sourceBranchPlaceholder": "例: main, origin/develop（省略可）",
    "copyWorkingChanges": "未コミット変更をコピーする",
    "path": "作成先パス（自動）",
    "creating": "作成中...",
    "create": "作成",
    "duplicating": "複製中...",
    "duplicate": "複製",
    "baseDirNotSet": "設定でワークツリー追加先ディレクトリを設定してください。",
    "sessionSource": "セッション引継ぎ元",
    "sessionSourceNone": "なし",
    "sessionSourceActive": "アクティブなワークツリー",
    "sessionSourceArchived": "アーカイブ済みワークツリー"
  }
}
</i18n>
