<script setup lang="ts">
import { useI18n } from "vue-i18n";

const { t } = useI18n();

defineProps<{
  totalCount: number;
}>();

defineEmits<{
  click: [];
}>();
</script>

<template>
  <button
    v-if="totalCount > 0"
    class="tray-btn"
    :class="{ 'tray-btn--pulse': totalCount > 0 }"
    :title="t('tooltip', { count: totalCount })"
    @click="$emit('click')"
  >
    <span class="pi pi-bell" style="font-size: 20px;" />
    <span class="tray-badge">{{ totalCount > 99 ? '99+' : totalCount }}</span>
  </button>
</template>

<style scoped>
.tray-btn {
  position: fixed;
  bottom: 16px;
  left: 16px;
  width: 48px;
  height: 48px;
  border-radius: 50%;
  background: #313244;
  border: 1px solid #45475a;
  color: #cdd6f4;
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  z-index: 100;
  transition: background 0.15s;
}

.tray-btn:hover {
  background: #45475a;
}

.tray-badge {
  position: absolute;
  top: 4px;
  right: 4px;
  min-width: 18px;
  height: 18px;
  border-radius: 9px;
  background: #f38ba8;
  color: #1e1e2e;
  font-size: 10px;
  font-weight: 700;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 0 3px;
  pointer-events: none;
}

@keyframes pulse {
  0%, 100% { box-shadow: 0 0 0 0 rgba(243, 139, 168, 0.4); }
  50% { box-shadow: 0 0 0 8px rgba(243, 139, 168, 0); }
}

.tray-btn--pulse {
  animation: pulse 2s ease-in-out infinite;
}
</style>

<i18n lang="json">
{
  "en": {
    "tooltip": "Notifications: {count}"
  },
  "ja": {
    "tooltip": "通知: {count} 件"
  }
}
</i18n>
