<script setup lang="ts">
defineProps<{
  items: string[];
  active: number;
}>();
</script>

<template>
  <div class="phase-strip">
    <div
      v-for="(item, i) in items"
      :key="i"
      class="phase-item"
      :class="{ 'phase-item--active': i === active }"
    >{{ item }}</div>
  </div>
</template>

<style scoped>
.phase-strip {
  display: flex;
  gap: 6px;
}

.phase-item {
  flex: 1;
  font-size: 10.5px;
  text-align: center;
  padding: 6px 4px;
  border-radius: 5px;
  line-height: 1.4;
  background: #181825;
  border: 1px solid #313244;
  color: #6c7086;
  transition: all 0.25s;
}

.phase-item--active {
  background: rgba(203, 166, 247, 0.12);
  border-color: #cba6f7;
  color: #cba6f7;
  font-weight: 600;
}
</style>
