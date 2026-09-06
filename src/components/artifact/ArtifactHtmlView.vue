<script setup lang="ts">
import { ref, computed, onMounted, onBeforeUnmount } from "vue";
import { buildHtmlSrcdoc } from "../../utils/htmlArtifactSrcdoc";
import { readArtifactNavigateMessage } from "../../utils/artifactFrameLink";

const props = defineProps<{
  content: string;
}>();

const emit = defineEmits<{
  (e: "navigate", href: string): void;
}>();

const frame = ref<HTMLIFrameElement | null>(null);

const srcdocHtml = computed(() => buildHtmlSrcdoc(props.content));

// sandbox の opaque origin では event.origin が "null" になり検証に使えないため、
// 送信元は contentWindow の同一性で判定する
function onMessage(event: MessageEvent) {
  const href = readArtifactNavigateMessage(event, frame.value);
  if (href) emit("navigate", href);
}

onMounted(() => window.addEventListener("message", onMessage));
onBeforeUnmount(() => window.removeEventListener("message", onMessage));
</script>

<template>
  <div class="html-view">
    <iframe
      ref="frame"
      :srcdoc="srcdocHtml"
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
