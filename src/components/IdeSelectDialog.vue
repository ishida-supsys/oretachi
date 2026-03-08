<script setup lang="ts">
import type { IdeInfo } from "../types/ide";
import { useI18n } from "vue-i18n";

const { t } = useI18n();

defineProps<{
  ides: IdeInfo[];
}>();

const emit = defineEmits<{
  select: [ide: IdeInfo];
  cancel: [];
}>();
</script>

<template>
  <div class="dialog-overlay" @click.self="emit('cancel')">
    <div class="dialog">
      <h3 class="dialog-title">{{ t('select') }}</h3>

      <div class="ide-list">
        <button
          v-for="ide in ides"
          :key="ide.id"
          class="ide-btn"
          @click="emit('select', ide)"
        >
          <span class="pi pi-desktop ide-icon" />
          <span class="ide-name">{{ ide.name }}</span>
        </button>
      </div>

      <div class="dialog-actions">
        <button class="btn-cancel" @click="emit('cancel')">{{ t('common.cancel') }}</button>
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
  width: 320px;
  max-width: 90vw;
}

.dialog-title {
  font-size: 16px;
  font-weight: 600;
  color: #cba6f7;
  margin: 0 0 16px;
}

.ide-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.ide-btn {
  display: flex;
  align-items: center;
  gap: 10px;
  background: #313244;
  color: #cdd6f4;
  border: 1px solid #45475a;
  border-radius: 6px;
  padding: 10px 14px;
  font-size: 14px;
  cursor: pointer;
  text-align: left;
  transition: background 0.15s;
}

.ide-btn:hover {
  background: #45475a;
}

.ide-icon {
  font-size: 16px;
  color: #89b4fa;
  flex-shrink: 0;
}

.ide-name {
  font-weight: 500;
}

.dialog-actions {
  display: flex;
  justify-content: flex-end;
  margin-top: 16px;
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
</style>

<i18n lang="json">
{
  "en": {
    "select": "Select IDE"
  },
  "ja": {
    "select": "IDE を選択"
  }
}
</i18n>
