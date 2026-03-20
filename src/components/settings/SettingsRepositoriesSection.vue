<script setup lang="ts">
import { invoke } from "@tauri-apps/api/core";
import { open, message } from "@tauri-apps/plugin-dialog";
import { useSettings } from "../../composables/useSettings";
import { useI18n } from "vue-i18n";

const { t } = useI18n();
const { settings, scheduleSave } = useSettings();

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

  // 重複チェック
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
      <label class="field-label">{{ t('repositories.label') }}</label>
      <button class="btn-primary" @click="addRepository">{{ t('repositories.add') }}</button>
    </div>
    <div class="repo-list">
      <div
        v-if="settings.repositories.length === 0"
        class="empty-state"
      >
        {{ t('repositories.empty') }}
      </div>
      <div
        v-for="repo in settings.repositories"
        :key="repo.id"
        class="repo-item"
      >
        <div class="repo-row-main">
          <span class="repo-name">{{ repo.name }}</span>
          <span class="repo-path">{{ repo.path }}</span>
          <button class="btn-remove" :disabled="hasWorktrees(repo.id)" :title="hasWorktrees(repo.id) ? t('repositories.hasWorktrees') : undefined" @click="removeRepository(repo.id)">×</button>
        </div>
        <div class="repo-row-script">
          <span class="script-label">{{ t('repositories.execScript') }}</span>
          <input
            class="text-input script-input"
            :value="repo.execScript ?? ''"
            readonly
            :placeholder="t('common.notConfigured')"
          />
          <button class="btn-secondary" @click="selectExecScript(repo.id)">{{ t('worktreeBaseDir.select') }}</button>
          <button
            v-if="repo.execScript"
            class="btn-secondary"
            @click="clearExecScript(repo.id)"
          >{{ t('common.clear') }}</button>
        </div>
      </div>
    </div>
  </div>
</template>

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
