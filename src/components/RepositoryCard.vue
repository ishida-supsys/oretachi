<script setup lang="ts">
import { ref, computed } from "vue";
import { useI18n } from "vue-i18n";
import { message } from "@tauri-apps/plugin-dialog";
import { applyPluginConfig } from "../composables/useSettings";
import type { Repository } from "../types/settings";
import type { Worktree } from "../types/worktree";
import TerminalThumbnail from "./TerminalThumbnail.vue";
import Popover from "primevue/popover";
import Badge from "primevue/badge";

const { t } = useI18n();

const props = defineProps<{
  repo: Repository;
  /** リポジトリ擬似ワークツリー。マイグレーション完了前は undefined になりうる */
  worktree?: Worktree;
  thumbnailUrls: Map<number, string>;
  artifactCount?: number;
  notificationCount?: number;
  hotkeyChar?: string;
  detached?: boolean;
  autoApproval?: boolean;
  /** 配下にワークツリーがあるか（登録解除の可否） */
  hasWorktrees?: boolean;
}>();

const emit = defineEmits<{
  selectTerminal: [terminalId: number];
  addTerminal: [worktreeId: string];
  openInIde: [worktreeId: string];
  openArtifacts: [repositoryId: string];
  moveToSubWindow: [worktreeId: string];
  moveToMainWindow: [worktreeId: string];
  focusSubWindow: [worktreeId: string];
  setHotkeyChar: [worktreeId: string];
  toggleAutoApproval: [worktreeId: string];
  configurePostAdd: [repositoryId: string];
  selectExecScript: [repositoryId: string];
  clearExecScript: [repositoryId: string];
  removeRepository: [repositoryId: string];
}>();

const menuRef = ref<InstanceType<typeof Popover> | null>(null);

function openMenu(event: MouseEvent) {
  menuRef.value?.toggle(event);
}

/** 擬似ワークツリー未生成のあいだはターミナル操作を出さない */
const worktreeId = computed(() => props.worktree?.id ?? null);

const terminalList = computed(() =>
  (props.worktree?.terminals ?? []).map((term) => ({
    id: term.id,
    title: term.title,
    imageUrl: props.thumbnailUrls.get(term.id) ?? null,
  })),
);

/** 追加設定と実行スクリプトの現在値を1行に畳んだサマリ */
const metaSummary = computed(() => {
  const parts: string[] = [];
  if (props.repo.packageManager) parts.push(props.repo.packageManager);
  if (props.repo.copyTargets?.length) {
    parts.push(t("postAdd.itemsSelected", { count: props.repo.copyTargets.length }));
  }
  if (props.repo.notificationHooks?.length) {
    parts.push(t("postAdd.hooksCount", { count: props.repo.notificationHooks.length }));
  }
  if (props.repo.pullBeforeAdd) parts.push(t("postAdd.pullBeforeAdd"));
  if (props.repo.execScript) {
    parts.push(props.repo.execScript.split(/[/\\]/).pop() ?? props.repo.execScript);
  }
  return parts.length > 0 ? parts.join(" | ") : t("postAdd.notConfigured");
});

function onThumbnailClick(terminalId: number) {
  if (props.detached && worktreeId.value) {
    emit("focusSubWindow", worktreeId.value);
  } else {
    emit("selectTerminal", terminalId);
  }
}

function onMoveWindow(id: string) {
  if (props.detached) {
    emit("moveToMainWindow", id);
  } else {
    emit("moveToSubWindow", id);
  }
}

function withMenuHidden<T>(fn: () => T): T {
  menuRef.value?.hide();
  return fn();
}

/**
 * リポジトリ root の .claude/settings.local.json に oretachi プラグイン設定を書き直す。
 * 手編集や削除で MCP / 通知が効かなくなったときの復旧口。
 * 親へ emit せずここで完結させる（他の状態に影響しない自己完結の操作のため）。
 */
async function reapplyPluginConfig() {
  try {
    await applyPluginConfig(props.repo.path, props.repo.name, props.repo.notificationHooks ?? []);
    await message(t("menu.reapplyPluginDone"), { kind: "info" });
  } catch (e) {
    await message(t("menu.reapplyPluginFailed", { error: e }), { kind: "error" });
  }
}
</script>

<template>
  <div
    class="worktree-card"
    :class="{ 'card-detached': detached, 'card-notified': notificationCount && notificationCount > 0 }"
  >
    <Badge
      v-if="notificationCount && notificationCount > 0"
      :value="notificationCount"
      severity="danger"
      class="notification-badge"
    />
    <div v-if="hotkeyChar || (artifactCount && artifactCount > 0)" class="top-left-badges">
      <div v-if="hotkeyChar" class="hotkey-badge">Alt+{{ hotkeyChar }}</div>
      <button
        v-if="artifactCount && artifactCount > 0"
        class="artifact-count-badge"
        :title="t('artifactsTooltip')"
        @click="emit('openArtifacts', repo.id)"
      >
        <span class="pi pi-box" style="font-size: 9px" />
        {{ artifactCount }}
      </button>
    </div>

    <div class="card-header">
      <div class="card-info">
        <div class="card-name-row">
          <span class="pi pi-folder card-repo-icon" :title="t('repositoryBadge')" />
          <span class="card-name">{{ repo.name }}</span>
          <span v-if="detached" class="card-detached-badge">{{ t('subWindowBadge') }}</span>
        </div>
        <span class="card-branch">{{ repo.path }}</span>
      </div>
      <div class="card-actions">
        <button
          v-if="worktreeId"
          class="btn-icon"
          :title="t('openInIde')"
          @click="emit('openInIde', worktreeId)"
        ><span class="pi pi-code" /></button>
        <button
          v-if="worktreeId && !detached"
          class="btn-icon"
          :title="t('addTerminal')"
          @click="emit('addTerminal', worktreeId)"
        >+</button>
        <button class="btn-icon" :title="t('menuTitle')" @click="openMenu($event)">
          <span class="pi pi-ellipsis-v" />
        </button>
      </div>
    </div>

    <div class="repo-meta" :title="metaSummary">{{ metaSummary }}</div>

    <div class="terminals-row">
      <div v-if="terminalList.length === 0" class="empty-terminals">
        {{ t('noTerminals') }}
      </div>
      <TerminalThumbnail
        v-for="item in terminalList"
        :key="item.id"
        :tab-id="item.id"
        :title="item.title"
        :image-url="item.imageUrl"
        :is-active="false"
        @click="onThumbnailClick(item.id)"
      />
    </div>

    <Popover ref="menuRef">
      <div class="popup-menu">
        <button class="popup-item" @click="withMenuHidden(() => emit('configurePostAdd', repo.id))">
          <span class="pi pi-cog" />
          {{ t('menu.postAddSettings') }}
        </button>
        <button class="popup-item" @click="withMenuHidden(() => emit('selectExecScript', repo.id))">
          <span class="pi pi-file" />
          {{ t('menu.selectExecScript') }}
        </button>
        <button
          v-if="repo.execScript"
          class="popup-item"
          @click="withMenuHidden(() => emit('clearExecScript', repo.id))"
        >
          <span class="pi pi-times" />
          {{ t('menu.clearExecScript') }}
        </button>
        <button class="popup-item" @click="withMenuHidden(() => emit('openArtifacts', repo.id))">
          <span class="pi pi-box" />
          {{ t('menu.openArtifacts') }}
        </button>
        <button class="popup-item" @click="withMenuHidden(reapplyPluginConfig)">
          <span class="pi pi-refresh" />
          {{ t('menu.reapplyPlugin') }}
        </button>
        <template v-if="worktreeId">
          <button
            class="popup-item"
            :style="autoApproval ? 'color: var(--p-green-400)' : ''"
            @click="withMenuHidden(() => emit('toggleAutoApproval', worktreeId!))"
          >
            <span :class="autoApproval ? 'pi pi-check-circle' : 'pi pi-circle'" />
            {{ t('menu.autoApproval') }}
          </button>
          <button class="popup-item" @click="withMenuHidden(() => emit('setHotkeyChar', worktreeId!))">
            <span class="pi pi-key" />
            {{ t('menu.setHotkey') }}
          </button>
          <button
            class="popup-item"
            @click="withMenuHidden(() => onMoveWindow(worktreeId!))"
          >
            <span :class="detached ? 'pi pi-window-maximize' : 'pi pi-external-link'" />
            {{ detached ? t('menu.moveToMainWindow') : t('menu.moveToSubWindow') }}
          </button>
        </template>
        <div class="popup-divider" />
        <button
          class="popup-item popup-item-danger"
          :disabled="hasWorktrees"
          :title="hasWorktrees ? t('menu.removeBlocked') : undefined"
          @click="withMenuHidden(() => emit('removeRepository', repo.id))"
        >
          <span class="pi pi-trash" />
          {{ t('menu.remove') }}
        </button>
      </div>
    </Popover>
  </div>
</template>

<style scoped>
/* 見た目はワークツリーカードに合わせる（クラス名も揃えて差分を追いやすくする） */
.worktree-card {
  position: relative;
  background: #181825;
  border: 1px solid #313244;
  border-radius: 8px;
  padding: 12px;
}

.notification-badge {
  position: absolute;
  top: -8px;
  right: -8px;
}

.top-left-badges {
  position: absolute;
  top: -8px;
  left: -8px;
  display: flex;
  gap: 4px;
  align-items: center;
}

.hotkey-badge {
  background: rgba(203, 166, 247, 0.2);
  border: 1px solid rgba(203, 166, 247, 0.5);
  border-radius: 4px;
  padding: 1px 6px;
  font-size: 10px;
  font-family: monospace;
  color: #cba6f7;
  white-space: nowrap;
}

.artifact-count-badge {
  display: flex;
  align-items: center;
  gap: 3px;
  background: rgba(137, 180, 250, 0.15);
  border: 1px solid rgba(137, 180, 250, 0.4);
  border-radius: 4px;
  padding: 1px 5px;
  font-size: 10px;
  color: #89b4fa;
  white-space: nowrap;
  cursor: pointer;
}

.artifact-count-badge:hover {
  border-color: #89b4fa;
}

.card-detached {
  border-color: #89b4fa;
  opacity: 0.7;
}

.card-notified {
  border-color: #f38ba8;
  animation: notification-pulse 2s ease-in-out infinite;
}

@keyframes notification-pulse {
  0%, 100% { box-shadow: 0 0 0 0 rgba(243, 139, 168, 0.2); }
  50%       { box-shadow: 0 0 8px 2px rgba(243, 139, 168, 0.3); }
}

.card-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 6px;
}

.card-info {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}

.card-name-row {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
}

.card-name {
  font-size: 14px;
  font-weight: 600;
  color: #cdd6f4;
  user-select: none;
}

.card-branch {
  font-size: 11px;
  color: #6c7086;
  word-break: break-all;
}

/* リポジトリの識別子。ワークグループバーのリポジトリチップと同じアイコン・色で揃える */
.card-repo-icon {
  font-size: 12px;
  color: #fab387;
  flex-shrink: 0;
}

.card-detached-badge {
  font-size: 10px;
  color: #89b4fa;
  background: rgba(137, 180, 250, 0.15);
  border-radius: 3px;
  padding: 1px 5px;
}

.card-actions {
  display: flex;
  gap: 6px;
}

.btn-icon {
  background: #313244;
  color: #cdd6f4;
  border: none;
  border-radius: 4px;
  width: 28px;
  height: 28px;
  font-size: 14px;
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
}

.btn-icon:hover {
  background: #45475a;
}

.repo-meta {
  font-size: 11px;
  color: #6c7086;
  margin-bottom: 10px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.terminals-row {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}

.empty-terminals {
  font-size: 12px;
  color: #6c7086;
  padding: 4px 0;
}

.popup-menu {
  display: flex;
  flex-direction: column;
  min-width: 180px;
}

.popup-item {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 12px;
  background: none;
  border: none;
  color: var(--p-text-color);
  font-size: 13px;
  cursor: pointer;
  border-radius: 4px;
  text-align: left;
  width: 100%;
}

.popup-item:hover {
  background: var(--p-content-hover-background);
}

.popup-item:disabled {
  opacity: 0.45;
  cursor: not-allowed;
}

.popup-item:disabled:hover {
  background: none;
}

.popup-item-danger {
  color: var(--p-red-400);
}

.popup-item-danger:not(:disabled):hover {
  background: color-mix(in srgb, var(--p-red-400) 15%, transparent);
}

.popup-divider {
  height: 1px;
  background: var(--p-content-border-color);
  margin: 4px 0;
}
</style>

<i18n lang="json">
{
  "en": {
    "openInIde": "Open in IDE",
    "addTerminal": "Add terminal",
    "menuTitle": "Menu",
    "noTerminals": "No terminals",
    "subWindowBadge": "Sub window",
    "repositoryBadge": "Repository",
    "artifactsTooltip": "Open artifacts saved to this repository",
    "postAdd": {
      "itemsSelected": "{count} selected",
      "hooksCount": "{count} hooks",
      "pullBeforeAdd": "fetch",
      "notConfigured": "Not configured"
    },
    "menu": {
      "postAddSettings": "Post-add settings",
      "selectExecScript": "Select exec script",
      "clearExecScript": "Clear exec script",
      "openArtifacts": "Artifacts",
      "reapplyPlugin": "Re-apply plugin settings",
      "reapplyPluginDone": "Wrote the oretachi plugin settings into .claude/settings.local.json.",
      "reapplyPluginFailed": "Failed to re-apply the plugin settings: {error}",
      "autoApproval": "Auto approval",
      "setHotkey": "Assign hotkey",
      "moveToSubWindow": "Move to sub window",
      "moveToMainWindow": "Move to main window",
      "remove": "Unregister repository",
      "removeBlocked": "Cannot unregister: worktrees exist"
    }
  },
  "ja": {
    "openInIde": "IDE で開く",
    "addTerminal": "ターミナルを追加",
    "menuTitle": "メニュー",
    "noTerminals": "ターミナルなし",
    "subWindowBadge": "サブウィンドウ",
    "repositoryBadge": "リポジトリ",
    "artifactsTooltip": "このリポジトリに保存したアーティファクトを開く",
    "postAdd": {
      "itemsSelected": "{count}件選択中",
      "hooksCount": "{count}件フック",
      "pullBeforeAdd": "fetch",
      "notConfigured": "未設定"
    },
    "menu": {
      "postAddSettings": "追加設定",
      "selectExecScript": "実行スクリプトを選択",
      "clearExecScript": "実行スクリプトを解除",
      "openArtifacts": "アーティファクト",
      "reapplyPlugin": "プラグイン設定を再適用",
      "reapplyPluginDone": ".claude/settings.local.json に oretachi プラグイン設定を書き込みました。",
      "reapplyPluginFailed": "プラグイン設定の再適用に失敗しました: {error}",
      "autoApproval": "自動承認",
      "setHotkey": "ホットキーを割り当て",
      "moveToSubWindow": "サブウィンドウへ移動",
      "moveToMainWindow": "メインウィンドウへ戻す",
      "remove": "リポジトリの登録を解除",
      "removeBlocked": "ワークツリーが存在するため解除できません"
    }
  }
}
</i18n>
