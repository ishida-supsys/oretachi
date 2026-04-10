<script setup lang="ts">
import { ref, computed, onMounted } from "vue";
import ArtifactCodeView from "./ArtifactCodeView.vue";
import { buildVendorHead, buildReactSrcdoc } from "../../utils/reactArtifactSrcdoc";

type VendorScripts = { react: string; reactDom: string; babel: string; tailwind: string };

// Promise キャッシュ: 同時マウント時も重複フェッチしない。失敗時は null にリセットしてリトライ可能にする。
let _vendorPromise: Promise<VendorScripts> | null = null;

function loadVendors(): Promise<VendorScripts> {
  if (!_vendorPromise) {
    const fetchText = async (url: string) => {
      const r = await fetch(url);
      if (!r.ok) throw new Error(`Failed to load ${url}: ${r.status} ${r.statusText}`);
      return r.text();
    };
    _vendorPromise = Promise.all([
      fetchText("/vendor/react.production.min.js"),
      fetchText("/vendor/react-dom.production.min.js"),
      import("@babel/standalone/babel.min.js?raw").then((m) => m.default),
      fetchText("/vendor/tailwindcss-browser.js"),
    ]).then(([react, reactDom, babel, tailwind]) => ({ react, reactDom, babel, tailwind }))
      .catch((e) => {
        _vendorPromise = null; // 失敗時はリトライ可能にする
        throw e;
      });
  }
  return _vendorPromise;
}

const props = defineProps<{
  content: string;
  modules?: Record<string, string>;
}>();

type Mode = "preview" | "code";
const mode = ref<Mode>("preview");
// コードビューで選択中のファイル: "" = エントリポイント、それ以外はモジュール名
const selectedFile = ref<string>("");

const vendorScripts = ref<VendorScripts | null>(null);
const vendorLoading = ref(true);
const vendorError = ref<string | null>(null);

onMounted(async () => {
  try {
    vendorScripts.value = await loadVendors();
  } catch (e) {
    vendorError.value = e instanceof Error ? e.message : String(e);
  } finally {
    vendorLoading.value = false;
  }
});

// ベンダーヘッド（~2MB）は vendorScripts が変化したときのみ再計算する
const vendorHead = computed(() => {
  if (!vendorScripts.value) return "";
  const { react, reactDom, babel, tailwind } = vendorScripts.value;
  return buildVendorHead(react, reactDom, babel, tailwind);
});

// content が変わっても vendorHead は再計算されない
const srcdocHtml = computed(() => {
  if (!vendorHead.value) return "";
  return buildReactSrcdoc(vendorHead.value, props.content, props.modules);
});

const moduleNames = computed(() => Object.keys(props.modules ?? {}));

// 同名ファイルが複数ある場合は親ディレクトリを含めて表示
const moduleLabel = computed(() => {
  const names = moduleNames.value;
  const basenames = names.map(n => n.split('/').pop() ?? n);
  return (name: string) => {
    const base = name.split('/').pop() ?? name;
    const isDuplicate = basenames.filter(b => b === base).length > 1;
    if (!isDuplicate) return base;
    const parts = name.split('/');
    return parts.length >= 2 ? `${parts[parts.length - 2]}/${base}` : name;
  };
});

const codeContent = computed(() =>
  selectedFile.value === "" ? props.content : (props.modules?.[selectedFile.value] ?? "")
);
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

    <div v-else class="code-area">
      <div v-if="moduleNames.length > 0" class="module-tabs">
        <button
          :class="{ active: selectedFile === '' }"
          @click="selectedFile = ''"
        >index</button>
        <button
          v-for="name in moduleNames"
          :key="name"
          :class="{ active: selectedFile === name }"
          :title="name"
          @click="selectedFile = name"
        >{{ moduleLabel(name) }}</button>
      </div>
      <ArtifactCodeView
        :content="codeContent"
        language="typescriptreact"
      />
    </div>
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

.code-area {
  flex: 1;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.module-tabs {
  display: flex;
  gap: 2px;
  padding: 4px 8px;
  background: #181825;
  border-bottom: 1px solid #313244;
  flex-shrink: 0;
  flex-wrap: wrap;
}

.module-tabs button {
  padding: 2px 10px;
  border: 1px solid #313244;
  border-radius: 3px;
  background: transparent;
  color: #6c7086;
  font-size: 11px;
  cursor: pointer;
  transition: background 0.12s, color 0.12s;
}

.module-tabs button:hover {
  background: #313244;
  color: #cdd6f4;
}

.module-tabs button.active {
  background: #313244;
  color: #cdd6f4;
  border-color: #45475a;
}
</style>
