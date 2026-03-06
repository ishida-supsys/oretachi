<script setup lang="ts">
defineProps<{
  tabId: number;
  title: string;
  imageUrl: string | null;
  isActive: boolean;
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
  padding: 4px;
  background: #1e1e2e;
  min-height: 35px;
  display: flex;
  align-items: center;
  justify-content: center;
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
