<script setup lang="ts">
import { useI18n } from "vue-i18n";

const { t } = useI18n();

const props = defineProps<{
  tabId: number;
  title: string;
  imageUrl: string | null;
  isActive: boolean;
  /** 復元待ちの AI エージェント種別。値があれば「未復元」バッジを出す（#328） */
  resumePendingAgent?: string | null;
}>();

const emit = defineEmits<{
  click: [tabId: number];
}>();
</script>

<template>
  <div
    class="thumbnail-card"
    :class="{ active: isActive }"
    @click="emit('click', tabId)"
  >
    <div class="canvas-wrapper">
      <img v-if="imageUrl" :src="imageUrl" class="thumbnail-img" />
      <div
        v-if="props.resumePendingAgent"
        class="resume-pending-badge"
        :title="t('resumePendingTooltip', { agent: props.resumePendingAgent })"
      >
        <span class="pi pi-history" />
      </div>
    </div>
    <div class="thumbnail-title">{{ title }}</div>
  </div>
</template>

<style scoped>
.thumbnail-card {
  background: #181825;
  border: 1px solid #313244;
  border-radius: 6px;
  cursor: pointer;
  overflow: hidden;
  transition: border-color 0.15s;
  user-select: none;
  width: 107px;
  flex: 0 0 107px;
}

.thumbnail-card:hover {
  border-color: #585b70;
}

.thumbnail-card.active {
  border-color: #cba6f7;
}

.canvas-wrapper {
  position: relative;
  padding: 4px;
  background: #1e1e2e;
  min-height: 35px;
  display: flex;
  align-items: center;
  justify-content: center;
}

.resume-pending-badge {
  position: absolute;
  top: 2px;
  right: 2px;
  display: flex;
  align-items: center;
  justify-content: center;
  width: 14px;
  height: 14px;
  border-radius: 50%;
  background: rgba(249, 226, 175, 0.9);
  color: #1e1e2e;
  font-size: 8px;
}

.thumbnail-img {
  display: block;
  width: 100%;
  image-rendering: pixelated;
}

.thumbnail-title {
  padding: 4px 8px;
  font-size: 11px;
  color: #6c7086;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  border-top: 1px solid #313244;
}

.thumbnail-card.active .thumbnail-title {
  color: #cba6f7;
}
</style>

<i18n lang="json">
{
  "en": {
    "resumePendingTooltip": "Not restored yet: open to resume {agent}"
  },
  "ja": {
    "resumePendingTooltip": "未復元: 開くと{agent}をresume"
  }
}
</i18n>
