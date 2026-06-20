<script setup lang="ts">
import { ref } from "vue";
import { useI18n } from "vue-i18n";
import { useWorkgroups } from "../composables/useWorkgroups";
import WorkgroupEditDialog from "./WorkgroupEditDialog.vue";
import type { Workgroup } from "../types/settings";

const { t } = useI18n();

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
}

function onChipClick(id: string) {
  // 既にアクティブなチップを再クリックで編集ダイアログを開く
  if (activeWorkgroupId.value === id) {
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
    <button
      v-for="g in groups"
      :key="g.id"
      class="wg-chip"
      :class="{ active: g.id === activeWorkgroupId, notified: notifiedGroupIds.has(g.id) }"
      :style="{ borderLeftColor: g.color || '#9399b2', ...(g.id === activeWorkgroupId && g.color ? { background: g.color + '30', borderColor: g.color } : {}) }"
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
      <span v-if="g.id === activeWorkgroupId" class="wg-edit" @click.stop="editingId = g.id">
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
    "chipTitle": "Click to switch / click again to edit"
  },
  "ja": {
    "addGroup": "グループを追加",
    "chipTitle": "クリックで切替 / 再クリックで編集"
  }
}
</i18n>
