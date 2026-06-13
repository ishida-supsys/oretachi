<script setup lang="ts">
import { open, message } from "@tauri-apps/plugin-dialog";
import { useI18n } from "vue-i18n";
import { useSettings } from "../../composables/useSettings";
import { useRepositoryActions } from "../../composables/useRepositoryActions";

const { t } = useI18n();
const { settings, scheduleSave } = useSettings();
const { addRepository: addRepositoryAction } = useRepositoryActions();

async function selectWorktreeBaseDir() {
  const selected = await open({ directory: true, multiple: false });
  if (typeof selected === "string") {
    settings.value.worktreeBaseDir = selected;
    scheduleSave();
  }
}

async function addRepository() {
  const result = await addRepositoryAction();
  if (result === "notARepo") {
    await message(t("error.notARepo"), { kind: "error" });
  } else if (result === "alreadyRegistered") {
    await message(t("error.alreadyRegistered"), { kind: "warning" });
  }
}

function hasWorktrees(repoId: string): boolean {
  return settings.value.worktrees.some((w) => w.repositoryId === repoId);
}

function removeRepository(id: string) {
  if (hasWorktrees(id)) return;
  settings.value.repositories = settings.value.repositories.filter((r) => r.id !== id);
  scheduleSave();
}
</script>

<template>
  <div class="step">
    <div class="section-label">{{ t('baseDirLabel') }}</div>
    <div class="row">
      <input
        class="text-input"
        :value="settings.worktreeBaseDir"
        readonly
        :placeholder="t('notConfigured')"
      />
      <button class="btn-secondary" @click="selectWorktreeBaseDir">{{ t('select') }}</button>
    </div>
    <p class="hint">{{ t('baseDirHint') }}</p>

    <div class="divider" />

    <div class="section-header">
      <span class="section-label">{{ t('repositoriesLabel') }}</span>
      <button class="btn-primary" @click="addRepository">{{ t('addRepository') }}</button>
    </div>

    <div class="repo-list">
      <div v-if="settings.repositories.length === 0" class="empty-state">
        {{ t('reposEmpty') }}
      </div>
      <div v-for="repo in settings.repositories" :key="repo.id" class="repo-item">
        <span class="repo-name">{{ repo.name }}</span>
        <span class="repo-path">{{ repo.path }}</span>
        <button
          class="btn-remove"
          :disabled="hasWorktrees(repo.id)"
          :title="hasWorktrees(repo.id) ? t('hasWorktrees') : undefined"
          @click="removeRepository(repo.id)"
        >&times;</button>
      </div>
    </div>

    <p class="note">{{ t('changeLater') }}</p>
  </div>
</template>

<style scoped>
.step {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.section-label {
  font-size: 12px;
  font-weight: 700;
  color: #a6adc8;
  letter-spacing: 0.5px;
}

.section-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.row {
  display: flex;
  gap: 8px;
  align-items: center;
}

.text-input {
  flex: 1;
  background: #313244;
  border: 1px solid #45475a;
  border-radius: 4px;
  padding: 8px 10px;
  font-size: 13px;
  color: #cdd6f4;
  outline: none;
}

.btn-primary {
  background: #cba6f7;
  color: #1e1e2e;
  border: none;
  border-radius: 4px;
  padding: 6px 12px;
  font-size: 12px;
  cursor: pointer;
  font-weight: 600;
  white-space: nowrap;
}

.btn-primary:hover {
  background: #b4befe;
}

.btn-secondary {
  background: #313244;
  color: #cdd6f4;
  border: 1px solid #45475a;
  border-radius: 4px;
  padding: 7px 14px;
  font-size: 12px;
  cursor: pointer;
  white-space: nowrap;
}

.btn-secondary:hover {
  background: #45475a;
}

.hint {
  margin: 0;
  font-size: 11.5px;
  color: #6c7086;
}

.divider {
  border-top: 1px solid #313244;
  margin: 10px 0;
}

.repo-list {
  border: 1px solid #313244;
  border-radius: 6px;
  overflow: hidden;
  max-height: 150px;
  overflow-y: auto;
}

.repo-item {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 8px 12px;
  border-bottom: 1px solid #313244;
  background: #181825;
}

.repo-item:last-child {
  border-bottom: none;
}

.repo-name {
  min-width: 110px;
  font-size: 13px;
  font-weight: 600;
  color: #cdd6f4;
}

.repo-path {
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 12px;
  color: #6c7086;
}

.btn-remove {
  background: transparent;
  color: #6c7086;
  border: none;
  font-size: 16px;
  cursor: pointer;
  padding: 0 4px;
  line-height: 1;
}

.btn-remove:hover:not(:disabled) {
  color: #f38ba8;
}

.btn-remove:disabled {
  color: #313244;
  cursor: not-allowed;
}

.empty-state {
  padding: 14px;
  text-align: center;
  color: #6c7086;
  font-size: 12px;
  background: #181825;
}

.note {
  margin: 6px 0 0;
  font-size: 11.5px;
  color: #6c7086;
  line-height: 1.7;
}
</style>

<i18n lang="json">
{
  "en": {
    "baseDirLabel": "Worktree Base Directory",
    "baseDirHint": "New worktrees will be created under this directory.",
    "notConfigured": "Not configured",
    "select": "Select...",
    "repositoriesLabel": "Repositories",
    "addRepository": "+ Add Repository",
    "reposEmpty": "No repositories registered",
    "hasWorktrees": "Cannot delete: worktrees exist",
    "changeLater": "You can proceed with these empty. Detailed per-repository settings (exec script etc.) are available later in Settings → Repositories.",
    "error": {
      "notARepo": "The selected folder is not a git repository.",
      "alreadyRegistered": "This repository is already registered."
    }
  },
  "ja": {
    "baseDirLabel": "ワークツリー追加先ディレクトリ",
    "baseDirHint": "ワークツリーを追加するとこのディレクトリ直下に作成されます。",
    "notConfigured": "未設定",
    "select": "選択...",
    "repositoriesLabel": "リポジトリ",
    "addRepository": "＋ リポジトリを追加",
    "reposEmpty": "リポジトリが登録されていません",
    "hasWorktrees": "ワークツリーが存在するため削除できません",
    "changeLater": "未設定でも次へ進めます。実行スクリプト等の詳細設定は 設定 → リポジトリ一覧 で後から構成できます。",
    "error": {
      "notARepo": "選択したフォルダは git リポジトリではありません。",
      "alreadyRegistered": "このリポジトリはすでに登録されています。"
    }
  }
}
</i18n>
