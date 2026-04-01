<script setup lang="ts">
import { useI18n } from "vue-i18n";

defineProps<{
  isWindowFocused: boolean;
  showMinimize?: boolean;
  showMaximize?: boolean;
}>();

defineEmits<{
  close: [];
  minimize: [];
  maximize: [];
}>();

const { t } = useI18n({ useScope: "global" });
</script>

<template>
  <div class="traffic-light-group flex items-center gap-[8px] pl-[10px] pr-2 shrink-0">
    <!-- 閉じる (赤) -->
    <button
      class="traffic-light"
      :class="isWindowFocused ? 'traffic-light-close' : 'traffic-light-inactive'"
      :title="t('close')"
      @click="$emit('close')"
    >
      <svg class="traffic-icon" viewBox="0 0 8 8" fill="none" xmlns="http://www.w3.org/2000/svg">
        <path d="M1 1L7 7M7 1L1 7" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/>
      </svg>
    </button>

    <!-- 最小化 (黄) -->
    <button
      v-if="showMinimize !== false"
      class="traffic-light"
      :class="isWindowFocused ? 'traffic-light-minimize' : 'traffic-light-inactive'"
      :title="t('minimize')"
      @click="$emit('minimize')"
    >
      <svg class="traffic-icon" viewBox="0 0 8 8" fill="none" xmlns="http://www.w3.org/2000/svg">
        <path d="M1 4H7" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/>
      </svg>
    </button>

    <!-- 最大化 (緑) -->
    <button
      v-if="showMaximize !== false"
      class="traffic-light"
      :class="isWindowFocused ? 'traffic-light-maximize' : 'traffic-light-inactive'"
      :title="t('maximize')"
      @click="$emit('maximize')"
    >
      <svg class="traffic-icon" viewBox="0 0 8 8" fill="none" xmlns="http://www.w3.org/2000/svg">
        <path d="M1 7L7 1M4.5 1H7V3.5M1 4.5V7H3.5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/>
      </svg>
    </button>
  </div>
</template>

<style scoped>
.traffic-light {
  width: 12px;
  height: 12px;
  border-radius: 50%;
  border: none;
  padding: 0;
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  transition: filter 0.1s;
}

.traffic-light:active {
  filter: brightness(0.85);
}

.traffic-light-close {
  background-color: #ff5f57;
}

.traffic-light-minimize {
  background-color: #febc2e;
}

.traffic-light-maximize {
  background-color: #28c840;
}

.traffic-light-inactive {
  background-color: #3a3a3c;
}

.traffic-icon {
  width: 8px;
  height: 8px;
  color: rgba(0, 0, 0, 0.5);
  opacity: 0;
  transition: opacity 0.1s;
}

.traffic-light-inactive .traffic-icon {
  display: none;
}

.traffic-light-group:hover .traffic-icon {
  opacity: 1;
}
</style>
