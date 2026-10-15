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
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { buildVendorHead, buildReactSrcdoc } from "./utils/reactArtifactSrcdoc";
import { loadVendors } from "./utils/reactArtifactVendors";
import { createCspNonce } from "./utils/artifactCspViolation";
import {
  ARTIFACT_BRIDGE_METHOD_FLUSHED,
  ARTIFACT_BRIDGE_METHOD_MCP_CALL,
  ARTIFACT_BRIDGE_METHOD_MEMORY_SET,
  readArtifactBridgeRequest,
  postArtifactBridgeResult,
  postArtifactBridgeMemoryChanged,
  postArtifactBridgeFlush,
  type ArtifactBridgeRequest,
} from "./utils/artifactMemory";

interface Bootstrap {
  worktreeId: string;
  artifactId: string;
  title: string;
  appName: string;
  content: string;
  modules: Record<string, string>;
  /** 保存済みのメモリー。memory 権限が承認されていないアプリには空 */
  memory: Record<string, unknown>;
}

/** コード更新が連続したとき（モジュールを何本も書き換える等）に 1 回へまとめる待ち */
const RELOAD_DEBOUNCE_MS = 300;
/** リロード前に、旧 iframe の保存（debounce 待ち・送信中）が終わるのを待つ上限 */
const FLUSH_TIMEOUT_MS = 2000;

const frame = ref<HTMLIFrameElement | null>(null);
const boot = ref<Bootstrap | null>(null);
const vendorHead = ref("");
const loadError = ref("");
/**
 * いま表示している srcdoc の nonce。リロードのたびに採番し直す。iframe を差し替えても
 * `contentWindow` は同一なので、前の文書の遅延メッセージはこの nonce の不一致で捨てる。
 */
const nonce = ref("");

function report(level: "info" | "warn" | "error", message: string) {
  void invoke("app_instance_report", { level, message }).catch(() => {});
}

// 書き込み・読み取りの可否と maxBytes の強制は Rust（`app_set_memory` / `app_get_memory`）が決める。
// ここは素通しするだけで、権限の判断はしない
const srcdoc = computed(() =>
  boot.value && vendorHead.value && nonce.value
    ? buildReactSrcdoc(vendorHead.value, boot.value.content, boot.value.modules, boot.value.memory, {
        nonce: nonce.value,
      })
    : "",
);

function errorText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

let flushWaiter: (() => void) | null = null;

// onMessage から同期で呼ぶこと（受信時点の nonce を捕捉するため）
async function handleBridgeRequest(request: ArtifactBridgeRequest) {
  // 応答に載せる nonce は**リクエストを受けた時点**のもの。返信時の nonce.value だと、待っている間に
  // リロードされたとき旧文書宛の応答が新文書の nonce を付けて届き、新文書の同じ requestId の待ちを解いてしまう
  const docNonce = nonce.value;
  const reply = (
    req: ArtifactBridgeRequest,
    outcome: { ok: true; result?: unknown } | { ok: false; error: string },
  ) => postArtifactBridgeResult(frame.value, req.requestId, outcome, docNonce);
  if (request.method === ARTIFACT_BRIDGE_METHOD_FLUSHED) {
    // 応答は返さない（リロード前の flush の完了通知）
    flushWaiter?.();
    return;
  }
  if (request.method === ARTIFACT_BRIDGE_METHOD_MEMORY_SET) {
    try {
      await invoke("app_set_memory", { memory: request.params.memory ?? null });
      reply(request, { ok: true });
    } catch (e) {
      // 拒否（権限なし・maxBytes 超過）の理由は Rust がデバッグログへ残している。アプリへはエラーで返す
      reply(request, { ok: false, error: errorText(e) });
    }
    return;
  }
  if (request.method === ARTIFACT_BRIDGE_METHOD_MCP_CALL) {
    const tool = request.params.tool;
    if (typeof tool !== "string" || tool === "") {
      reply(request, { ok: false, error: "callTool には tool 名が必要です" });
      return;
    }
    const p = request.params.params;
    try {
      const result = await invoke<string>("app_call_mcp_tool", {
        tool,
        params: p && typeof p === "object" && !Array.isArray(p) ? p : {},
      });
      reply(request, { ok: true, result });
    } catch (e) {
      // 拒否の理由は Rust がデバッグログへ残している。ここはアプリへ返すだけ
      reply(request, { ok: false, error: errorText(e) });
    }
    return;
  }
  reply(request, { ok: false, error: `このアプリでは使えない API です: ${request.method}` });
}

// sandbox の opaque origin では event.origin が "null" になるので、送信元は contentWindow の同一性で判定する。
// それだけでは前の文書の遅延メッセージを弾けないので、nonce も突き合わせる
function onMessage(event: MessageEvent) {
  if (!nonce.value) return;
  const request = readArtifactBridgeRequest(event, frame.value, nonce.value);
  if (request) void handleBridgeRequest(request);
}

/** 旧 iframe の debounce 待ち・送信中の保存を送り切らせ、完了（かタイムアウト）まで待つ */
async function flushFrame(): Promise<void> {
  if (!frame.value?.contentWindow) return;
  const done = new Promise<boolean>((resolve) => {
    const timer = setTimeout(() => {
      flushWaiter = null;
      resolve(false);
    }, FLUSH_TIMEOUT_MS);
    flushWaiter = () => {
      clearTimeout(timer);
      flushWaiter = null;
      resolve(true);
    };
  });
  postArtifactBridgeFlush(frame.value, nonce.value);
  if (!(await done)) {
    report("warn", "リロード前のメモリー保存が時間内に終わらなかったため、直前の入力が反映されない可能性があります");
  }
}

let reloading = false;
let reloadAgain = false;
let reloadTimer: ReturnType<typeof setTimeout> | null = null;

function scheduleReload() {
  if (reloadTimer !== null) clearTimeout(reloadTimer);
  reloadTimer = setTimeout(() => {
    reloadTimer = null;
    void doReload();
  }, RELOAD_DEBOUNCE_MS);
}

/**
 * フルリロード（状態を保つ HMR ではない。メモリー以外の JS の状態は失われる）。
 * 順序が肝: **flush → 読み直し → 差し替え**。flush より先に読み直すと、debounce 待ちの値が落ちる。
 */
async function doReload() {
  if (reloading) {
    reloadAgain = true;
    return;
  }
  reloading = true;
  try {
    await flushFrame();
    const next = await invoke<Bootstrap>("app_instance_bootstrap");
    boot.value = next;
    nonce.value = createCspNonce();
    report("info", "リロードしました");
  } catch (e) {
    // 読み直せなければ旧 iframe のまま動かし続ける（壊れた書き込みの途中を表示しない）
    report("error", `リロードに失敗（旧版のまま継続）: ${errorText(e)}`);
  } finally {
    reloading = false;
    if (reloadAgain) {
      reloadAgain = false;
      scheduleReload();
    }
  }
}

/** MCP の `artifact_store` が外からメモリーを書き換えたので、iframe へ押し込む（応答の無い一方向通知） */
async function pushExternalMemory() {
  const docNonce = nonce.value;
  if (!docNonce) return;
  try {
    const memory = await invoke<Record<string, unknown>>("app_get_memory");
    // 読んでいる間にリロードされたなら捨てる（新文書は新しい bootstrap で最新を読んでいる）
    if (docNonce !== nonce.value) return;
    postArtifactBridgeMemoryChanged(frame.value, memory, docNonce);
  } catch {
    // memory 権限が無いアプリには押し込まない（Rust が拒否する）
  }
}

const unlisteners: UnlistenFn[] = [];

onMounted(async () => {
  window.addEventListener("message", onMessage);
  document.body.style.margin = "0";
  document.body.style.background = "#1e1e2e";
  try {
    // リスナーは bootstrap の前に張る。読み込み中に来たリロード依頼を取りこぼさないため
    // （宛先はこのウィンドウだけ。Rust が `emit_to(label)` で送る）
    unlisteners.push(await getCurrentWebviewWindow().listen("app-instance-reload", scheduleReload));
    // メモリーの外部更新は全体イベント。自分のアーティファクトのものだけ拾う
    unlisteners.push(
      await listen<{ scope: string; scopeId: string; artifactId: string }>("artifact-state-changed", (event) => {
        const p = event.payload;
        const b = boot.value;
        if (b && p.scope === "worktree" && p.scopeId === b.worktreeId && p.artifactId === b.artifactId) {
          void pushExternalMemory();
        }
      }),
    );
    const [b, vendors] = await Promise.all([
      invoke<Bootstrap>("app_instance_bootstrap"),
      loadVendors(),
    ]);
    boot.value = b;
    vendorHead.value = buildVendorHead(vendors.react, vendors.reactDom, vendors.babel, vendors.tailwind);
    nonce.value = createCspNonce();
  } catch (e) {
    loadError.value = errorText(e);
    report("error", `実行ウィンドウの読み込みに失敗: ${loadError.value}`);
  }
});

onBeforeUnmount(() => {
  window.removeEventListener("message", onMessage);
  if (reloadTimer !== null) clearTimeout(reloadTimer);
  for (const un of unlisteners) un();
});
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
