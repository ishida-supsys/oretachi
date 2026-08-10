<script setup lang="ts">
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useWorkgroups } from "../composables/useWorkgroups";
import { useHomePanel } from "../composables/useHomePanel";
import { useSettings } from "../composables/useSettings";
import WorkgroupEditDialog from "./WorkgroupEditDialog.vue";
import HomeSettingsDialog from "./HomeSettingsDialog.vue";
import type { Workgroup } from "../types/settings";

const { t } = useI18n();
const { listMode } = useHomePanel();
const { settings } = useSettings();

const repositoryCount = computed(() => settings.value.repositories.length);
const repositoryActive = computed(() => listMode.value === "repository");
const showHomeSettings = ref(false);

// リポジトリチップも通常のグループチップと同じ挙動:
// 非アクティブならリポジトリ一覧へ切替、アクティブ時の再クリック（と ✎）で設定ダイアログを開く
function onRepositoryChipClick() {
  if (repositoryActive.value) {
    showHomeSettings.value = true;
  } else {
    listMode.value = "repository";
  }
}

const emit = defineEmits<{
  removeWorkgroup: [groupId: string];
}>();

const {
  groups,
  activeWorkgroupId,
  displayName,
  worktreeCount,
  notifiedGroupIds,
  addWorkgroup,
  updateWorkgroup,
  reorderWorkgroup,
} = useWorkgroups();

const editingId = ref<string | null>(null);
const draggingId = ref<string | null>(null);

function select(id: string) {
  activeWorkgroupId.value = id;
  // リポジトリ一覧を出している最中にグループを選んだらワークツリー一覧へ戻す
  listMode.value = "worktree";
}

function onChipClick(id: string) {
  // リポジトリ一覧の表示中は、アクティブなチップでも「まず一覧を戻す」のが自然なので編集に入らない
  if (activeWorkgroupId.value === id && !repositoryActive.value) {
    editingId.value = id;
  } else {
    select(id);
  }
}

function onAdd() {
  const g = addWorkgroup();
  editingId.value = g.id;
}

function editingGroup(): Workgroup | undefined {
  return groups.value.find((g) => g.id === editingId.value);
}

function onSave(patch: Partial<Workgroup>) {
  if (editingId.value) updateWorkgroup(editingId.value, patch);
  editingId.value = null;
}

function onRemove() {
  const id = editingId.value;
  editingId.value = null;
  if (id) emit("removeWorkgroup", id);
}

// 並び替え (D&D): 並び替えは drop 時に一度だけ確定する
function onDragStart(id: string, event: DragEvent) {
  draggingId.value = id;
  if (event.dataTransfer) {
    event.dataTransfer.effectAllowed = "move";
    event.dataTransfer.setData("text/plain", id);
  }
}
function onDragOver(event: DragEvent) {
  event.preventDefault();
  if (event.dataTransfer) event.dataTransfer.dropEffect = "move";
}
function onDrop(id: string) {
  if (draggingId.value && draggingId.value !== id) {
    reorderWorkgroup(draggingId.value, id);
  }
  draggingId.value = null;
}
function onDragEnd() {
  draggingId.value = null;
}
</script>

<template>
  <div class="workgroup-bar">
    <!-- リポジトリ一覧チップ: グループチップと同一デザイン。
         アイコン・固定色の左罫・直後の区切り線でグループと区別する -->
    <button
      class="wg-chip wg-chip-repository"
      :class="{ active: repositoryActive }"
      :title="t('repositoryChipTitle')"
      @click="onRepositoryChipClick"
    >
      <i class="pi pi-folder wg-repo-icon" />
      <!-- ワークツリー一覧を見ているあいだは場所を取らないようアイコンと設定ボタンだけにし、
           リポジトリ一覧を開いているあいだだけラベルと件数を出す -->
      <template v-if="repositoryActive">
        <span class="wg-name">{{ t('repositoryChip') }}</span>
        <span class="wg-count">{{ repositoryCount }}</span>
      </template>
      <span class="wg-edit" :title="t('homeSettings')" @click.stop="showHomeSettings = true">
        <i class="pi pi-pencil" style="font-size: 10px" />
      </span>
    </button>
    <div class="wg-sep" />

    <button
      v-for="g in groups"
      :key="g.id"
      class="wg-chip"
      :class="{ active: g.id === activeWorkgroupId && !repositoryActive, notified: notifiedGroupIds.has(g.id) }"
      :style="{ borderLeftColor: g.color || '#9399b2', ...(g.id === activeWorkgroupId && !repositoryActive && g.color ? { background: g.color + '30', borderColor: g.color } : {}) }"
      draggable="true"
      :title="t('chipTitle')"
      @click="onChipClick(g.id)"
      @dragstart="onDragStart(g.id, $event)"
      @dragover="onDragOver($event)"
      @drop="onDrop(g.id)"
      @dragend="onDragEnd"
    >
      <span class="wg-name">{{ displayName(g) }}</span>
      <span class="wg-count">{{ worktreeCount(g.id) }}</span>
      <span v-if="g.id === activeWorkgroupId && !repositoryActive" class="wg-edit" @click.stop="editingId = g.id">
        <i class="pi pi-pencil" style="font-size: 10px" />
      </span>
    </button>

    <button class="wg-add" :title="t('addGroup')" @click="onAdd">
      <i class="pi pi-plus" style="font-size: 12px" />
    </button>

    <WorkgroupEditDialog
      v-if="editingGroup()"
      :group="editingGroup()!"
      :can-delete="groups.length > 1"
      @save="onSave"
      @remove="onRemove"
      @cancel="editingId = null"
    />

    <HomeSettingsDialog v-if="showHomeSettings" @close="showHomeSettings = false" />
  </div>
</template>

<style scoped>
.workgroup-bar {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  align-items: center;
  flex: 1;
  min-width: 0;
}

.wg-chip {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 5px 10px;
  border-radius: 7px;
  border: 1px solid #313244;
  border-left: 4px solid #9399b2;
  background: #181825;
  color: #a6adc8;
  font-size: 12px;
  font-weight: 600;
  cursor: pointer;
  white-space: nowrap;
  transition: background 0.12s, color 0.12s;
}

.wg-chip:hover {
  background: #232336;
}

.wg-chip.active {
  background: #313244;
  color: #cdd6f4;
  border-color: #585b70;
}

.wg-chip.notified {
  box-shadow: 0 0 0 2px #f38ba8;
  animation: wg-notification-pulse 2s ease-in-out infinite;
}

@keyframes wg-notification-pulse {
  0%, 100% {
    box-shadow: 0 0 0 2px rgba(243, 139, 168, 0.6);
  }
  50% {
    box-shadow: 0 0 0 2px rgba(243, 139, 168, 1), 0 0 8px 2px rgba(243, 139, 168, 0.3);
  }
}

.wg-count {
  background: #45475a;
  color: #bac2de;
  border-radius: 8px;
  padding: 0 6px;
  font-size: 10px;
  font-weight: 700;
  min-width: 16px;
  text-align: center;
}

.wg-chip.active .wg-count {
  background: #585b70;
  color: #cdd6f4;
}

/* リポジトリチップ: 寸法・角丸・フォントは .wg-chip をそのまま使い、
   左罫の固定色とアイコンだけでグループチップと区別する */
.wg-chip-repository {
  border-left-color: #fab387;
}

.wg-chip-repository.active {
  background: #fab38730;
  border-color: #fab387;
  color: #cdd6f4;
}

.wg-repo-icon {
  font-size: 11px;
  color: #fab387;
}

.wg-sep {
  width: 1px;
  height: 18px;
  background: #45475a;
  margin: 0 2px;
  flex-shrink: 0;
}

.wg-edit {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  color: #9399b2;
  margin-left: 2px;
}

.wg-edit:hover {
  color: #cba6f7;
}

.wg-add {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 28px;
  height: 28px;
  border-radius: 7px;
  border: 1px dashed #45475a;
  background: transparent;
  color: #6c7086;
  cursor: pointer;
}

.wg-add:hover {
  color: #cba6f7;
  border-color: #cba6f7;
}
</style>

<i18n lang="json">
{
  "en": {
    "addGroup": "Add group",
    "chipTitle": "Click to switch / click again to edit",
    "repositoryChip": "Repositories",
    "repositoryChipTitle": "Show repositories",
    "homeSettings": "Home & management agent settings"
  },
  "ja": {
    "addGroup": "グループを追加",
    "chipTitle": "クリックで切替 / 再クリックで編集",
    "repositoryChip": "リポジトリ",
    "repositoryChipTitle": "リポジトリ一覧を表示",
    "homeSettings": "ホーム・管理エージェント設定"
  }
}
</i18n>
