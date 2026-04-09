<script setup lang="ts">
import { ref, computed } from "vue";
import type { Worktree } from "../types/worktree";
import { useI18n } from "vue-i18n";

const { t } = useI18n();
import TerminalThumbnail from "./TerminalThumbnail.vue";
import Popover from "primevue/popover";
import Badge from "primevue/badge";

const props = defineProps<{
  worktree: Worktree;
  thumbnailUrls: Map<number, string>;
  detached?: boolean;
  notificationCount?: number;
  hotkeyChar?: string;
  artifactCount?: number;
  loading?: boolean;
  loadingText?: string;
  cancellable?: boolean;
  autoApproval?: boolean;
  aiJudging?: boolean;
}>();

const emit = defineEmits<{
  selectTerminal: [terminalId: number];
  dragStart: [worktreeId: string, event: DragEvent];
  dragEnd: [];
  addTerminal: [worktreeId: string];
  removeWorktree: [worktreeId: string];
  cancelRemove: [worktreeId: string];
  openInIde: [worktreeId: string];
  moveToSubWindow: [worktreeId: string];
  moveToMainWindow: [worktreeId: string];
  focusSubWindow: [worktreeId: string];
  setHotkeyChar: [worktreeId: string];
  toggleAutoApproval: [worktreeId: string];
  cancelAiJudging: [worktreeId: string];
  openArtifacts: [worktreeId: string];
  duplicateWorktree: [worktreeId: string];
}>();

const menuRef = ref<InstanceType<typeof Popover> | null>(null);

function openMenu(event: MouseEvent) {
  menuRef.value?.toggle(event);
}

function onMoveWindow() {
  menuRef.value?.hide();
  if (props.detached) {
    emit("moveToMainWindow", props.worktree.id);
  } else {
    emit("moveToSubWindow", props.worktree.id);
  }
}

function onOpenArtifacts() {
  menuRef.value?.hide();
  emit("openArtifacts", props.worktree.id);
}

function onDuplicate() {
  menuRef.value?.hide();
  emit("duplicateWorktree", props.worktree.id);
}

function onDelete() {
  menuRef.value?.hide();
  emit("removeWorktree", props.worktree.id);
}

function onSetHotkeyChar() {
  menuRef.value?.hide();
  emit("setHotkeyChar", props.worktree.id);
}

function onThumbnailClick(terminalId: number) {
  if (props.detached) {
    emit("focusSubWindow", props.worktree.id);
  } else {
    emit("selectTerminal", terminalId);
  }
}

const terminalList = computed(() =>
  props.worktree.terminals.map((t) => ({
    id: t.id,
    title: t.title,
    imageUrl: props.thumbnailUrls.get(t.id) ?? null,
  }))
);
</script>

<template>
  <div class="worktree-card" :class="{ 'card-detached': detached, 'card-notified': notificationCount && notificationCount > 0 }">
    <Badge v-if="notificationCount && notificationCount > 0" :value="notificationCount" severity="danger" class="notification-badge" />
    <div v-if="hotkeyChar || (artifactCount && artifactCount > 0)" class="top-left-badges">
      <div v-if="hotkeyChar" class="hotkey-badge">Alt+{{ hotkeyChar }}</div>
      <div v-if="artifactCount && artifactCount > 0" class="artifact-count-badge">
        <span class="pi pi-box" style="font-size: 9px" />
        {{ artifactCount }}
      </div>
    </div>
    <div class="card-header">
      <div class="card-info">
        <span
          class="card-name"
          draggable="true"
          @dragstart.stop="$emit('dragStart', worktree.id, $event)"
          @dragend.stop="$emit('dragEnd')"
        >{{ worktree.name }}</span>
        <span class="card-branch">{{ worktree.branchName }}</span>
        <span v-if="detached" class="card-detached-badge">{{ t('subWindowBadge') }}</span>
        <button
          v-if="aiJudging"
          class="ai-judging-badge"
          @click="emit('cancelAiJudging', worktree.id)"
        >
          <span class="pi pi-spin pi-spinner" style="font-size: 10px" />
          {{ t('aiJudgingBadge') }}
        </button>
      </div>
      <div class="card-actions">
        <button
          class="btn-icon"
          :title="t('openInIde')"
          :disabled="loading"
          @click="emit('openInIde', worktree.id)"
        ><span class="pi pi-code" /></button>
        <button
          v-if="!detached"
          class="btn-icon"
          :title="t('addTerminal')"
          :disabled="loading"
          @click="emit('addTerminal', worktree.id)"
        >+</button>
        <button
          class="btn-icon"
          title="メニュー"
          :disabled="loading"
          @click="openMenu($event)"
        ><span class="pi pi-ellipsis-v" /></button>
      </div>
    </div>

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
        <button
          class="popup-item"
          :style="autoApproval ? 'color: var(--p-green-400)' : ''"
          :disabled="loading"
          @click="emit('toggleAutoApproval', worktree.id)"
        >
          <span :class="autoApproval ? 'pi pi-check-circle' : 'pi pi-circle'" />
          {{ t('menu.autoApproval') }}
        </button>
        <button class="popup-item" :disabled="loading" @click="onSetHotkeyChar">
          <span class="pi pi-key" />
          {{ t('menu.setHotkey') }}
        </button>
        <button class="popup-item" :disabled="loading" @click="onOpenArtifacts">
          <span class="pi pi-box" />
          {{ t('menu.openArtifacts') }}
        </button>
        <button class="popup-item" :disabled="loading" @click="onMoveWindow">
          <span :class="detached ? 'pi pi-window-maximize' : 'pi pi-external-link'" />
          {{ detached ? t('menu.moveToMainWindow') : t('menu.moveToSubWindow') }}
        </button>
        <button class="popup-item" :disabled="loading" @click="onDuplicate">
          <span class="pi pi-copy" />
          {{ t('menu.duplicate') }}
        </button>
        <div class="popup-divider" />
        <button class="popup-item popup-item-danger" :disabled="loading" @click="onDelete">
          <span class="pi pi-trash" />
          {{ t('menu.delete') }}
        </button>
      </div>
    </Popover>

    <!-- ローディングオーバーレイ -->
    <div v-if="loading" class="loading-overlay">
      <span class="pi pi-spinner pi-spin loading-icon" />
      <span class="loading-text">{{ loadingText ?? t('deletingText') }}</span>
      <button
        v-if="cancellable"
        class="cancel-remove-btn"
        @click.stop="emit('cancelRemove', worktree.id)"
      >
        {{ t('cancelRemove') }}
      </button>
    </div>
  </div>
</template>

<style scoped>
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
  margin-bottom: 10px;
}

.card-info {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}

.card-name {
  font-size: 14px;
  font-weight: 600;
  color: #cdd6f4;
  cursor: grab;
  user-select: none;
}

.card-name:active {
  cursor: grabbing;
}

.card-branch {
  font-size: 11px;
  color: #6c7086;
  word-break: break-all;
}

.card-detached-badge {
  font-size: 10px;
  color: #89b4fa;
  background: rgba(137, 180, 250, 0.15);
  border-radius: 3px;
  padding: 1px 5px;
}

.ai-judging-badge {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  font-size: 10px;
  color: #1e1e2e;
  background: #f9e2af;
  border: none;
  border-radius: 3px;
  padding: 2px 6px;
  cursor: pointer;
  font-weight: 600;
}

.ai-judging-badge:hover {
  background: #f5c842;
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

.popup-item-danger {
  color: var(--p-red-400);
}

.popup-item-danger:hover {
  background: color-mix(in srgb, var(--p-red-400) 15%, transparent);
}

.popup-divider {
  height: 1px;
  background: var(--p-content-border-color);
  margin: 4px 0;
}

.loading-overlay {
  position: absolute;
  inset: 0;
  background: rgba(30, 30, 46, 0.8);
  border-radius: 8px;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 8px;
}

.loading-icon {
  font-size: 20px;
  color: #cba6f7;
}

.loading-text {
  font-size: 12px;
  color: #a6adc8;
}

.cancel-remove-btn {
  margin-top: 4px;
  padding: 4px 12px;
  font-size: 11px;
  color: #cdd6f4;
  background: rgba(243, 139, 168, 0.15);
  border: 1px solid rgba(243, 139, 168, 0.4);
  border-radius: 4px;
  cursor: pointer;
  transition: background 0.15s;
}

.cancel-remove-btn:hover {
  background: rgba(243, 139, 168, 0.3);
}
</style>

<i18n lang="json">
{
  "en": {
    "subWindowBadge": "Sub window",
    "autoApprovalBadge": "Auto approval",
    "aiJudgingBadge": "AI judging",
    "openInIde": "Open in IDE",
    "openArtifacts": "Open artifacts",
    "addTerminal": "Add terminal",
    "noTerminals": "No terminals",
    "deletingText": "Deleting...",
    "cancelRemove": "Cancel",
    "menu": {
      "autoApproval": "Auto approval",
      "setHotkey": "Assign hotkey",
      "openArtifacts": "Artifacts",
      "moveToSubWindow": "Move to sub window",
      "moveToMainWindow": "Move to main window",
      "duplicate": "Duplicate",
      "delete": "Delete"
    }
  },
  "ja": {
    "subWindowBadge": "サブウィンドウ",
    "autoApprovalBadge": "自動承認",
    "aiJudgingBadge": "AI判定中",
    "openInIde": "IDE で開く",
    "openArtifacts": "アーティファクト",
    "addTerminal": "ターミナルを追加",
    "noTerminals": "ターミナルがありません",
    "deletingText": "削除中...",
    "cancelRemove": "キャンセル",
    "menu": {
      "autoApproval": "自動承認",
      "setHotkey": "ホットキー割り当て",
      "openArtifacts": "アーティファクト",
      "moveToSubWindow": "サブウィンドウに移動",
      "moveToMainWindow": "メインウィンドウに戻す",
      "duplicate": "複製",
      "delete": "削除"
    }
  }
}
</i18n>
