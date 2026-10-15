<script setup lang="ts">
/**
 * アプリ形式アーティファクトの**常駐ウィンドウ（`appinst-*`）のシェル**。oretachi 自身のコード。
 *
 * - アプリのコードはここではなく、中の `sandbox="allow-scripts"` iframe の中でだけ動く
 *   （アプリのコードがトップレベル文書になってはならない。このウィンドウは default.json の
 *   glob に含まれず、専用 capability（`capabilities/appinst.json`）だけを持つ）。
 * - **インスタンスを引数で指定しない。** どのアプリかは Rust がこのウィンドウのラベルから決める。
 *   ブリッジの `mcp.call` も `app_call_mcp_tool`（ラベルで識別）へ素通しするだけで、
 *   許可・拒否の判断はここではしない（唯一の関門は Rust）。
 * - 閉じる = 隠す（Rust が prevent_close で hide にする）。インスタンスは継続する。
 */
import { ref, computed, onMounted, onBeforeUnmount } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { buildVendorHead, buildReactSrcdoc } from "./utils/reactArtifactSrcdoc";
import { loadVendors } from "./utils/reactArtifactVendors";
import {
  ARTIFACT_BRIDGE_METHOD_MCP_CALL,
  readArtifactBridgeRequest,
  postArtifactBridgeResult,
  type ArtifactBridgeRequest,
} from "./utils/artifactMemory";

interface Bootstrap {
  worktreeId: string;
  artifactId: string;
  title: string;
  appName: string;
  content: string;
  modules: Record<string, string>;
}

const frame = ref<HTMLIFrameElement | null>(null);
const boot = ref<Bootstrap | null>(null);
const vendorHead = ref("");
const loadError = ref("");

function report(level: "info" | "warn" | "error", message: string) {
  void invoke("app_instance_report", { level, message }).catch(() => {});
}

// メモリー API は #371。ここでは常に空で始め、保存は拒否する（誤って通常の経路へ流さない）
const srcdoc = computed(() =>
  boot.value && vendorHead.value
    ? buildReactSrcdoc(vendorHead.value, boot.value.content, boot.value.modules, {})
    : "",
);

async function handleBridgeRequest(request: ArtifactBridgeRequest) {
  if (request.method === ARTIFACT_BRIDGE_METHOD_MCP_CALL) {
    const tool = request.params.tool;
    if (typeof tool !== "string" || tool === "") {
      postArtifactBridgeResult(frame.value, request.requestId, {
        ok: false,
        error: "callTool には tool 名が必要です",
      });
      return;
    }
    const p = request.params.params;
    try {
      const result = await invoke<string>("app_call_mcp_tool", {
        tool,
        params: p && typeof p === "object" && !Array.isArray(p) ? p : {},
      });
      postArtifactBridgeResult(frame.value, request.requestId, { ok: true, result });
    } catch (e) {
      // 拒否の理由は Rust がデバッグログへ残している。ここはアプリへ返すだけ
      postArtifactBridgeResult(frame.value, request.requestId, {
        ok: false,
        error: e instanceof Error ? e.message : String(e),
      });
    }
    return;
  }
  postArtifactBridgeResult(frame.value, request.requestId, {
    ok: false,
    error: `このアプリでは使えない API です: ${request.method}`,
  });
}

// sandbox の opaque origin では event.origin が "null" になるので、送信元は contentWindow の同一性で判定する
function onMessage(event: MessageEvent) {
  const request = readArtifactBridgeRequest(event, frame.value);
  if (request) void handleBridgeRequest(request);
}

onMounted(async () => {
  window.addEventListener("message", onMessage);
  document.body.style.margin = "0";
  document.body.style.background = "#1e1e2e";
  try {
    const [b, vendors] = await Promise.all([
      invoke<Bootstrap>("app_instance_bootstrap"),
      loadVendors(),
    ]);
    boot.value = b;
    vendorHead.value = buildVendorHead(vendors.react, vendors.reactDom, vendors.babel, vendors.tailwind);
  } catch (e) {
    loadError.value = e instanceof Error ? e.message : String(e);
    report("error", `実行ウィンドウの読み込みに失敗: ${loadError.value}`);
  }
});

onBeforeUnmount(() => window.removeEventListener("message", onMessage));
</script>

<template>
  <div class="appinst-shell">
    <div v-if="loadError" class="shell-error">{{ loadError }}</div>
    <iframe
      v-else-if="srcdoc"
      ref="frame"
      :srcdoc="srcdoc"
      sandbox="allow-scripts"
      class="app-frame"
    />
    <div v-else class="shell-loading">…</div>
  </div>
</template>

<style scoped>
.appinst-shell {
  position: fixed;
  inset: 0;
  display: flex;
  background: #1e1e2e;
}
.app-frame {
  flex: 1;
  border: none;
  background: #fff;
}
.shell-error,
.shell-loading {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 16px;
  color: #f38ba8;
  font: 13px monospace;
  text-align: center;
  white-space: pre-wrap;
}
.shell-loading {
  color: #6c7086;
}
</style>
