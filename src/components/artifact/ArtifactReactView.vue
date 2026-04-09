<script setup lang="ts">
import { ref, computed, onMounted } from "vue";
import ArtifactCodeView from "./ArtifactCodeView.vue";
import { buildReactSrcdoc } from "../../utils/reactArtifactSrcdoc";

// モジュールスコープキャッシュ: コンポーネントの再マウント時に再フェッチしない
let _vendorCache: { react: string; reactDom: string; babel: string } | null = null;

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

const vendorError = ref<string | null>(null);

onMounted(async () => {
  if (!_vendorCache) {
    try {
      const fetchText = async (url: string) => {
        const r = await fetch(url);
        if (!r.ok) throw new Error(`Failed to load ${url}: ${r.status} ${r.statusText}`);
        return r.text();
      };
      const [react, reactDom, babel] = await Promise.all([
        fetchText("/vendor/react.production.min.js"),
        fetchText("/vendor/react-dom.production.min.js"),
        import("@babel/standalone/babel.min.js?raw").then((m) => m.default),
      ]);
      _vendorCache = { react, reactDom, babel };
    } catch (e) {
      vendorError.value = e instanceof Error ? e.message : String(e);
    }
  }
  vendorScripts.value = _vendorCache;
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
      <div v-else-if="vendorError" class="vendor-error">
        <span class="pi pi-exclamation-triangle" />
        {{ vendorError }}
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

.vendor-error {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 8px;
  color: #f38ba8;
  font-size: 13px;
  font-family: monospace;
  padding: 16px;
  text-align: center;
}

.react-iframe {
  flex: 1;
  border: none;
  background: #fff;
}
</style>
