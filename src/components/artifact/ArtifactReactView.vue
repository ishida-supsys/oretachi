<script setup lang="ts">
import { ref, computed, watch, onMounted, onBeforeUnmount } from "vue";
import ArtifactCodeView from "./ArtifactCodeView.vue";
import { buildVendorHead, buildReactSrcdoc } from "../../utils/reactArtifactSrcdoc";
import { readArtifactNavigateMessage } from "../../utils/artifactFrameLink";
import {
  ARTIFACT_BRIDGE_METHOD_MEMORY_SET,
  ARTIFACT_BRIDGE_METHOD_MCP_CALL,
  readArtifactBridgeRequest,
  postArtifactBridgeResult,
  postArtifactBridgeMemoryChanged,
  type ArtifactBridgeRequest,
} from "../../utils/artifactMemory";

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
  /** メモリーの初期値（サイドカーの `memory`）。初回レンダリングの復元にだけ使う */
  memory?: Record<string, unknown>;
  /** メモリーの保存。解決/棄却がそのまま iframe 内の setMemory の Promise になる */
  saveMemory?: (memory: Record<string, unknown>) => Promise<void>;
  /**
   * MCP ツール呼び出し。解決値がそのまま iframe 内の `callTool` の戻り値になる。
   * ホワイトリストとスコープの強制は Rust 側が行うので、ここは素通しでよい。
   */
  callTool?: (tool: string, params: Record<string, unknown>) => Promise<string>;
}>();

const emit = defineEmits<{
  (e: "navigate", href: string): void;
  /** メモリーの保存が失敗したとき。アーティファクト側は入力を受け付け続けるので UI で知らせる */
  (e: "memory-error", message: string): void;
}>();

const frame = ref<HTMLIFrameElement | null>(null);

type Mode = "preview" | "code";
const mode = ref<Mode>("preview");

/**
 * メモリーは iframe の初期値としてしか使わない。
 * 保存のたびに srcdoc を作り直すと iframe がリロードされて入力中のフォームが飛ぶため、
 * 取り込み直すのは iframe がどうせ作り直されるときだけにする。
 *
 * 作り直されるのは content が変わったときだけ（mode 切替では iframe を v-show で残す。
 * 破棄すると Preview へ戻った iframe がマウント時点の古いメモリーで起動し、
 * 次の setMemory がそれを丸ごと書き戻して保存済みの入力を消してしまう）。
 * アーティファクトの切り替えとリセットは、親が `:key` を進めて作り直す。
 */
const initialMemory = ref<Record<string, unknown>>({ ...(props.memory ?? {}) });
watch(
  () => props.content,
  () => {
    initialMemory.value = { ...(props.memory ?? {}) };
  },
);

/**
 * 外からストアが書き換わったことを iframe へ知らせる（MCP の `artifact_store` 由来）。
 *
 * 押し込まないと、iframe は起動時のスナップショットを持ち続け、次の 1 入力で
 * 自分の状態を丸ごと書き戻して外からの書き込みを消してしまう。
 * srcdoc を作り直す手も使えない（作り直すと入力中のフォームが飛ぶ）。
 */
function pushMemory(memory: Record<string, unknown>) {
  postArtifactBridgeMemoryChanged(frame.value, memory);
}

defineExpose({ pushMemory });

/**
 * iframe からの MCP ツール呼び出し。
 *
 * 許可されているツールとスコープの強制は Rust 側（`call_tool_for_artifact`）が唯一の関門。
 * ここで一覧を持たないのは、フロントの一覧を「権限」と誤解させないため。
 * 失敗はメモリー保存と違って親へ emit しない（アーティファクト側が Promise を
 * 受け取る前提の API なので、握り潰されたら iframe の unhandledrejection で表に出る）。
 */
async function handleMcpCall(request: ArtifactBridgeRequest) {
  const tool = request.params.tool;
  if (typeof tool !== "string" || tool === "") {
    postArtifactBridgeResult(frame.value, request.requestId, {
      ok: false,
      error: "callTool には tool 名が必要です",
    });
    return;
  }
  if (!props.callTool) {
    postArtifactBridgeResult(frame.value, request.requestId, {
      ok: false,
      error: "MCP ツール呼び出しはこのアーティファクトでは使えません",
    });
    return;
  }
  const toolParams = request.params.params;
  try {
    const result = await props.callTool(
      tool,
      toolParams && typeof toolParams === "object" && !Array.isArray(toolParams)
        ? (toolParams as Record<string, unknown>)
        : {},
    );
    postArtifactBridgeResult(frame.value, request.requestId, { ok: true, result });
  } catch (e) {
    const error = e instanceof Error ? e.message : String(e);
    console.error(`MCP ツール呼び出しに失敗: ${tool}`, e);
    postArtifactBridgeResult(frame.value, request.requestId, { ok: false, error });
  }
}

/** iframe からのブリッジ要求を処理して応答を返す */
async function handleBridgeRequest(request: ArtifactBridgeRequest) {
  if (request.method === ARTIFACT_BRIDGE_METHOD_MCP_CALL) {
    await handleMcpCall(request);
    return;
  }
  if (request.method !== ARTIFACT_BRIDGE_METHOD_MEMORY_SET) {
    postArtifactBridgeResult(frame.value, request.requestId, {
      ok: false,
      error: `unsupported method: ${request.method}`,
    });
    return;
  }

  const memory = request.params.memory;
  if (!memory || typeof memory !== "object" || Array.isArray(memory)) {
    postArtifactBridgeResult(frame.value, request.requestId, {
      ok: false,
      error: "memory must be a plain object",
    });
    return;
  }
  if (!props.saveMemory) {
    postArtifactBridgeResult(frame.value, request.requestId, {
      ok: false,
      error: "memory is not available for this artifact",
    });
    return;
  }

  try {
    await props.saveMemory(memory as Record<string, unknown>);
    postArtifactBridgeResult(frame.value, request.requestId, { ok: true });
  } catch (e) {
    // 上限超過などで保存が落ちても iframe は楽観更新した値を表示し続ける。
    // アーティファクト側が Promise を捨てていると誰も気づけないので親にも上げる
    const error = e instanceof Error ? e.message : String(e);
    console.error("set_artifact_memory failed", e);
    emit("memory-error", error);
    postArtifactBridgeResult(frame.value, request.requestId, { ok: false, error });
  }
}

// sandbox の opaque origin では event.origin が "null" になり検証に使えないため、
// 送信元は contentWindow の同一性で判定する
function onMessage(event: MessageEvent) {
  const href = readArtifactNavigateMessage(event, frame.value);
  if (href) {
    emit("navigate", href);
    return;
  }
  const request = readArtifactBridgeRequest(event, frame.value);
  if (request) void handleBridgeRequest(request);
}

onMounted(() => window.addEventListener("message", onMessage));
onBeforeUnmount(() => window.removeEventListener("message", onMessage));

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
  return buildReactSrcdoc(vendorHead.value, props.content, props.modules, initialMemory.value);
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

    <!-- v-show で残すのは、iframe を作り直すとアーティファクト内の React state と
         debounce 中のメモリー保存が飛ぶため。Code タブは重いので必要になってから作る -->
    <div v-show="mode === 'preview'" class="preview-area">
      <div v-if="vendorLoading" class="vendor-loading">
        <span class="pi pi-spin pi-spinner" />
      </div>
      <div v-else-if="vendorError" class="vendor-error">
        <span class="pi pi-exclamation-triangle" />
        {{ vendorError }}
      </div>
      <iframe
        v-else
        ref="frame"
        :srcdoc="srcdocHtml"
        sandbox="allow-scripts"
        allow="fullscreen"
        allowfullscreen
        class="react-iframe"
      />
    </div>

    <div v-if="mode === 'code'" class="code-area">
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
