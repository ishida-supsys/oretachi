<script setup lang="ts">
import { ref, computed, onMounted } from "vue";
import ArtifactCodeView from "./ArtifactCodeView.vue";
import { buildReactSrcdoc } from "../../utils/reactArtifactSrcdoc";

const props = defineProps<{
  content: string;
}>();

type Mode = "preview" | "code";
const mode = ref<Mode>("preview");

const vendorScripts = ref<{
  react: string;
  reactDom: string;
  babel: string;
} | null>(null);
const vendorLoading = ref(true);

onMounted(async () => {
  const [react, reactDom, babel] = await Promise.all([
    fetch("/vendor/react.production.min.js").then((r) => r.text()),
    fetch("/vendor/react-dom.production.min.js").then((r) => r.text()),
    import("@babel/standalone/babel.min.js?raw").then((m) => m.default),
  ]);
  vendorScripts.value = { react, reactDom, babel };
  vendorLoading.value = false;
});

const srcdocHtml = computed(() => {
  if (!vendorScripts.value) return "";
  const { react, reactDom, babel } = vendorScripts.value;
  return buildReactSrcdoc(react, reactDom, babel, props.content);
});
</script>

<template>
  <div class="react-view">
    <div class="react-toolbar">
      <button
        :class="{ active: mode === 'preview' }"
        @click="mode = 'preview'"
      >
        <span class="pi pi-play" />
        Preview
      </button>
      <button
        :class="{ active: mode === 'code' }"
        @click="mode = 'code'"
      >
        <span class="pi pi-code" />
        Code
      </button>
    </div>

    <div v-if="mode === 'preview'" class="preview-area">
      <div v-if="vendorLoading" class="vendor-loading">
        <span class="pi pi-spin pi-spinner" />
      </div>
      <iframe
        v-else
        :srcdoc="srcdocHtml"
        sandbox="allow-scripts"
        class="react-iframe"
      />
    </div>

    <ArtifactCodeView
      v-else
      :content="content"
      language="typescriptreact"
    />
  </div>
</template>

<style scoped>
.react-view {
  height: 100%;
  width: 100%;
  display: flex;
  flex-direction: column;
}

.react-toolbar {
  display: flex;
  gap: 4px;
  padding: 6px 12px;
  background: #181825;
  border-bottom: 1px solid #313244;
  flex-shrink: 0;
}

.react-toolbar button {
  display: flex;
  align-items: center;
  gap: 5px;
  padding: 4px 12px;
  border: 1px solid #313244;
  border-radius: 4px;
  background: transparent;
  color: #6c7086;
  font-size: 12px;
  cursor: pointer;
  transition: background 0.12s, color 0.12s;
}

.react-toolbar button:hover {
  background: #313244;
  color: #cdd6f4;
}

.react-toolbar button.active {
  background: #313244;
  color: #cdd6f4;
  border-color: #45475a;
}

.react-toolbar button .pi {
  font-size: 11px;
}

.preview-area {
  flex: 1;
  display: flex;
  overflow: hidden;
}

.vendor-loading {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  color: #6c7086;
  font-size: 18px;
}

.react-iframe {
  flex: 1;
  border: none;
  background: #fff;
}
</style>
