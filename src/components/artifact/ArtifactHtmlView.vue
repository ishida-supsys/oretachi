<script setup lang="ts">
defineProps<{
  content: string;
}>();
</script>

<template>
  <div class="html-view">
    <iframe
      :srcdoc="content"
      sandbox="allow-scripts"
      class="html-iframe"
    />
  </div>
</template>

<style scoped>
.html-view {
  height: 100%;
  width: 100%;
  display: flex;
}

.html-iframe {
  flex: 1;
  border: none;
  background: #fff;
  border-radius: 4px;
}
</style>
