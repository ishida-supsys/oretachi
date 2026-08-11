<script setup lang="ts">
/**
 * アーティファクトボタン／バッジのアイコン。
 * URL アーティファクトを持つ時は、既存のキューブの右下にリンクアイコンを重ねて示す。
 * 大きさは呼び出し元が font-size で決め、重ねるリンクは em 追従で縮尺する。
 */
defineProps<{
  hasUrl?: boolean;
}>();
</script>

<template>
  <span class="artifact-icon">
    <span class="pi pi-box artifact-icon-base" />
    <span v-if="hasUrl" class="pi pi-link artifact-icon-overlay" />
  </span>
</template>

<style scoped>
.artifact-icon {
  position: relative;
  display: inline-block;
  line-height: 1;
}

/* .pi の font-size: 1rem を打ち消し、呼び出し元の font-size に追従させる */
.artifact-icon .artifact-icon-base {
  display: block;
  font-size: 1em;
  line-height: 1;
}

.artifact-icon .artifact-icon-overlay {
  position: absolute;
  right: -0.3em;
  bottom: -0.28em;
  font-size: 0.72em;
  line-height: 1;
  /* キューブの線と重なっても輪郭が潰れないよう暗いハローを敷く。
     --bg-base は透過ウィンドウで transparent になりハローが消えるため、
     カタッピチン系の暗色をリテラルで置く */
  text-shadow:
    0 0 2px #1e1e2e,
    0 0 2px #1e1e2e;
}
</style>
