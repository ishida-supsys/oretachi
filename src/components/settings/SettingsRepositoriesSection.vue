<script setup lang="ts">
import { invoke } from "@tauri-apps/api/core";
import { open, message } from "@tauri-apps/plugin-dialog";
import { useI18n } from "vue-i18n";
import { useSettings } from "../../composables/useSettings";
import { usePostAddSettings } from "../../composables/usePostAddSettings";
import PostAddSettingsDialog from "../PostAddSettingsDialog.vue";

const { t } = useI18n();
const { settings, scheduleSave } = useSettings();
const {
  showCopyDialog,
  copyDialogRepoPath,
  copyDialogCurrentTargets,
  openCopyDialog,
  onCopyDialogConfirm,
  clearCopyTargets,
} = usePostAddSettings();

async function addRepository() {
  const selected = await open({ directory: true, multiple: false });
  if (typeof selected !== "string") return;

  try {
    const valid = await invoke<boolean>("git_validate_repo", { path: selected });
    if (!valid) {
      await message(t("error.notARepo"), { kind: "error" });
      return;
    }
  } catch {
    await message(t("error.notARepo"), { kind: "error" });
    return;
  }

  const name = selected.split(/[/\\]/).pop() ?? selected;

  if (settings.value.repositories.some((r) => r.path === selected)) {
    await message(t("error.alreadyRegistered"), { kind: "warning" });
    return;
  }

  settings.value.repositories.push({
    id: selected,
    name,
    path: selected,
  });
  scheduleSave();
}

function removeRepository(id: string) {
  settings.value.repositories = settings.value.repositories.filter(
    (r) => r.id !== id
  );
  scheduleSave();
}

function hasWorktrees(repoId: string): boolean {
  return settings.value.worktrees.some((w) => w.repositoryId === repoId);
}

async function selectExecScript(repoId: string) {
  const selected = await open({
    multiple: false,
    filters: [{ name: "Scripts", extensions: ["ps1", "sh"] }],
  });
  if (typeof selected !== "string") return;

  const repo = settings.value.repositories.find((r) => r.id === repoId);
  if (!repo) return;

  repo.execScript = selected;
  scheduleSave();
}

function clearExecScript(repoId: string) {
  const repo = settings.value.repositories.find((r) => r.id === repoId);
  if (!repo) return;

  repo.execScript = undefined;
  scheduleSave();
}


</script>

<template>
  <div class="field-group">
    <div class="field-header">
      <label class="field-label">{{ t("repositories.label") }}</label>
      <button class="btn-primary" @click="addRepository">
        {{ t("repositories.add") }}
      </button>
    </div>

    <div class="repo-list">
      <div v-if="settings.repositories.length === 0" class="empty-state">
        {{ t("repositories.empty") }}
      </div>

      <div
        v-for="repo in settings.repositories"
        :key="repo.id"
        class="repo-item"
      >
        <div class="repo-row-main">
          <span class="repo-name">{{ repo.name }}</span>
          <span class="repo-path">{{ repo.path }}</span>
          <button
            class="btn-remove"
            :disabled="hasWorktrees(repo.id)"
            :title="
              hasWorktrees(repo.id)
                ? t('repositories.hasWorktrees')
                : undefined
            "
            @click="removeRepository(repo.id)"
          >&times;</button>
        </div>

        <div class="repo-row-script">
          <div class="row-col row-col-left">
            <span class="script-label">{{ t("postAdd.label") }}</span>
            <span class="copy-summary">{{ t("postAdd.itemsSelected", { count: repo.copyTargets?.length ?? 0 }) }}</span>
            <button class="btn-secondary" @click="openCopyDialog(repo.id)">
              {{ t("postAdd.configure") }}
            </button>
            <button
              v-if="repo.copyTargets?.length"
              class="btn-secondary"
              @click="clearCopyTargets(repo.id)"
            >
              {{ t("common.clear") }}
            </button>
          </div>
          <div class="row-col row-col-right">
            <span class="script-label">{{ t("repositories.execScript") }}</span>
            <input
              class="text-input script-input"
              :value="repo.execScript ?? ''"
              readonly
              :placeholder="t('common.notConfigured')"
            />
            <button class="btn-secondary" @click="selectExecScript(repo.id)">
              {{ t("worktreeBaseDir.select") }}
            </button>
            <button
              v-if="repo.execScript"
              class="btn-secondary"
              @click="clearExecScript(repo.id)"
            >
              {{ t("common.clear") }}
            </button>
          </div>
        </div>
      </div>
    </div>
  </div>

  <PostAddSettingsDialog
    v-if="showCopyDialog"
    :repo-path="copyDialogRepoPath"
    :current-targets="copyDialogCurrentTargets"
    @confirm="onCopyDialogConfirm"
    @cancel="showCopyDialog = false"
  />
</template>

<style scoped>
.field-group {
  margin-bottom: 24px;
}

.field-label {
  display: block;
  font-size: 13px;
  color: #a6adc8;
  margin-bottom: 8px;
}

.field-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 8px;
}

.text-input {
  flex: 1;
  background: #313244;
  border: 1px solid #45475a;
  border-radius: 4px;
  padding: 6px 10px;
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
  padding: 6px 12px;
  font-size: 12px;
  cursor: pointer;
  white-space: nowrap;
}

.btn-secondary:hover {
  background: #45475a;
}

.repo-list {
  border: 1px solid #313244;
  border-radius: 6px;
  overflow: hidden;
}

.repo-item {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 10px 12px;
  border-bottom: 1px solid #313244;
  background: #181825;
}

.repo-item:last-child {
  border-bottom: none;
}

.repo-row-main {
  display: flex;
  align-items: center;
  gap: 12px;
}

.repo-row-script {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 8px;
}

.row-col {
  display: flex;
  align-items: center;
  gap: 8px;
}

.repo-name {
  min-width: 120px;
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

.script-label {
  white-space: nowrap;
  font-size: 11px;
  color: #6c7086;
}

.copy-summary {
  font-size: 11px;
  color: #a6adc8;
  white-space: nowrap;
}

.script-input {
  font-size: 12px;
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

.btn-remove:hover {
  color: #f38ba8;
}

.btn-remove:disabled {
  color: #313244;
  cursor: not-allowed;
}

.empty-state {
  padding: 16px;
  text-align: center;
  color: #6c7086;
  font-size: 13px;
  background: #181825;
}
</style>

<i18n lang="json">
{
  "en": {
    "repositories": {
      "label": "Repositories",
      "add": "+ Add",
      "empty": "No repositories registered",
      "hasWorktrees": "Cannot delete: worktrees exist",
      "execScript": "Exec script"
    },
    "postAdd": {
      "label": "Post-add",
      "configure": "Configure",
      "itemsSelected": "{count} selected"
    },
    "error": {
      "notARepo": "The selected folder is not a git repository.",
      "alreadyRegistered": "This repository is already registered."
    },
    "common": {
      "notConfigured": "Not configured",
      "clear": "Clear"
    },
    "worktreeBaseDir": {
      "select": "Select"
    }
  },
  "ja": {
    "repositories": {
      "label": "リポジトリ一覧",
      "add": "+ 追加",
      "empty": "リポジトリが登録されていません",
      "hasWorktrees": "ワークツリーが存在するため削除できません",
      "execScript": "実行スクリプト"
    },
    "postAdd": {
      "label": "追加後",
      "configure": "設定",
      "itemsSelected": "{count}件選択中"
    },
    "error": {
      "notARepo": "選択したフォルダは git リポジトリではありません。",
      "alreadyRegistered": "このリポジトリはすでに登録されています。"
    },
    "common": {
      "notConfigured": "未設定",
      "clear": "クリア"
    },
    "worktreeBaseDir": {
      "select": "選択"
    }
  }
}
</i18n>
