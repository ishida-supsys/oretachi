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
 *
 * このシェルは**サブウィンドウ（`appinst-<n>-w<k>`、透明・枠なしも可）でも同じものが動く**。
 * 中身は同じアプリで、`bootstrap.subWindow` が非 null のとき（サブウィンドウ）だけ、アプリへ
 * `oretachi.subWindow.info`（`{ id, params }`）を渡す。サブウィンドウはユーザーが閉じれば実際に閉じる
 * （Rust 側。インスタンスは止まらない）。API の関門は常駐ウィンドウと同じ（唯一の関門は Rust）。
 */
import { ref, computed, onMounted, onBeforeUnmount } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { buildVendorHead, buildReactSrcdoc } from "./utils/reactArtifactSrcdoc";
import { loadVendors } from "./utils/reactArtifactVendors";
import { createCspNonce } from "./utils/artifactCspViolation";
import {
  ARTIFACT_BRIDGE_METHOD_DB_CLOSE,
  ARTIFACT_BRIDGE_METHOD_NET_FETCH,
  ARTIFACT_BRIDGE_METHOD_DB_EXECUTE,
  ARTIFACT_BRIDGE_METHOD_DB_QUERY,
  ARTIFACT_BRIDGE_METHOD_AI_TERMINAL_PREFIX,
  ARTIFACT_BRIDGE_METHOD_FLUSHED,
  ARTIFACT_BRIDGE_METHOD_FS_LIST,
  ARTIFACT_BRIDGE_METHOD_FS_READ,
  ARTIFACT_BRIDGE_METHOD_FS_REMOVE,
  ARTIFACT_BRIDGE_METHOD_FS_WRITE,
  ARTIFACT_BRIDGE_METHOD_MCP_CALL,
  ARTIFACT_BRIDGE_METHOD_MEMORY_SET,
  ARTIFACT_BRIDGE_METHOD_SUB_WINDOW_PREFIX,
  readArtifactBridgeRequest,
  postArtifactBridgeResult,
  postArtifactBridgeMemoryChanged,
  postArtifactBridgeFlush,
  postArtifactBridgeAiTerminalEvent,
  postArtifactBridgeSubWindowEvent,
  type ArtifactBridgeRequest,
} from "./utils/artifactMemory";
import { aiTerminalCommandFor } from "./utils/aiTerminalBridge";
import { subWindowCommandFor } from "./utils/subWindowBridge";

interface Bootstrap {
  worktreeId: string;
  artifactId: string;
  title: string;
  appName: string;
  content: string;
  modules: Record<string, string>;
  /** 保存済みのメモリー。memory 権限が承認されていないアプリには空 */
  memory: Record<string, unknown>;
  /** サブウィンドウのシェルだけ非 null。常駐ウィンドウは null */
  subWindow: { id: string; params: unknown; transparent: boolean; frameless: boolean } | null;
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
        ...(boot.value.subWindow
          ? {
              subWindow: { id: boot.value.subWindow.id, params: boot.value.subWindow.params },
              transparentBackground: boot.value.subWindow.transparent,
            }
          : {}),
      })
    : "",
);

/** 透明なサブウィンドウでは、シェルの背景も透明にする（webview の透過が見えるように） */
function applyWindowBackground(b: Bootstrap) {
  const transparent = b.subWindow?.transparent === true;
  const bg = transparent ? "transparent" : "#1e1e2e";
  document.documentElement.style.background = transparent ? "transparent" : "";
  document.body.style.background = bg;
}

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
  if (
    request.method === ARTIFACT_BRIDGE_METHOD_FS_READ ||
    request.method === ARTIFACT_BRIDGE_METHOD_FS_WRITE ||
    request.method === ARTIFACT_BRIDGE_METHOD_FS_LIST ||
    request.method === ARTIFACT_BRIDGE_METHOD_FS_REMOVE
  ) {
    const path = request.params.path;
    if (typeof path !== "string" || path === "") {
      reply(request, { ok: false, error: "fs にはパス（worktree://… / artifact://…）が必要です" });
      return;
    }
    try {
      // 許可・拒否の判断は Rust（`app_fs_*`）。拒否の理由はデバッグログへ残る。ここは素通しするだけ
      let result: unknown;
      if (request.method === ARTIFACT_BRIDGE_METHOD_FS_READ) {
        result = await invoke("app_fs_read", { path, encoding: String(request.params.encoding ?? "utf8") });
      } else if (request.method === ARTIFACT_BRIDGE_METHOD_FS_WRITE) {
        result = await invoke("app_fs_write", {
          path,
          data: typeof request.params.data === "string" ? request.params.data : "",
          encoding: String(request.params.encoding ?? "utf8"),
          createDirs: request.params.createDirs === true,
        });
      } else if (request.method === ARTIFACT_BRIDGE_METHOD_FS_LIST) {
        result = await invoke("app_fs_list", { path });
      } else {
        result = await invoke("app_fs_remove", { path });
      }
      reply(request, { ok: true, result });
    } catch (e) {
      reply(request, { ok: false, error: errorText(e) });
    }
    return;
  }
  if (
    request.method === ARTIFACT_BRIDGE_METHOD_DB_QUERY ||
    request.method === ARTIFACT_BRIDGE_METHOD_DB_EXECUTE ||
    request.method === ARTIFACT_BRIDGE_METHOD_DB_CLOSE
  ) {
    const path = request.params.path;
    if (typeof path !== "string" || path === "") {
      reply(request, { ok: false, error: "db にはパス（worktree://… / artifact://…）が必要です" });
      return;
    }
    try {
      // 許可・拒否の判断は Rust（`app_db_*`）。拒否の理由はデバッグログへ残る。ここは素通しするだけ
      let result: unknown;
      if (request.method === ARTIFACT_BRIDGE_METHOD_DB_CLOSE) {
        result = await invoke("app_db_close", { path });
      } else {
        const sql = typeof request.params.sql === "string" ? request.params.sql : "";
        const params = Array.isArray(request.params.params) ? request.params.params : [];
        result = await invoke(
          request.method === ARTIFACT_BRIDGE_METHOD_DB_QUERY ? "app_db_query" : "app_db_execute",
          { path, sql, params },
        );
      }
      reply(request, { ok: true, result });
    } catch (e) {
      reply(request, { ok: false, error: errorText(e) });
    }
    return;
  }
  if (request.method === ARTIFACT_BRIDGE_METHOD_NET_FETCH) {
    const url = request.params.url;
    if (typeof url !== "string" || url === "") {
      reply(request, { ok: false, error: "net.fetch には URL が必要です" });
      return;
    }
    try {
      // 許可・拒否の判断（URL プレフィックス・リダイレクト・IP・上限）は Rust（`app_net_fetch`）。
      // 拒否の理由はデバッグログへ残る。ここは素通しするだけ
      const p = request.params;
      const result = await invoke("app_net_fetch", {
        url,
        init: {
          method: typeof p.method === "string" ? p.method : undefined,
          headers: Array.isArray(p.headers) ? p.headers : [],
          body: typeof p.body === "string" ? p.body : undefined,
          bodyEncoding: typeof p.bodyEncoding === "string" ? p.bodyEncoding : undefined,
          responseType: typeof p.responseType === "string" ? p.responseType : undefined,
        },
      });
      reply(request, { ok: true, result });
    } catch (e) {
      reply(request, { ok: false, error: errorText(e) });
    }
    return;
  }
  // 専用サブウィンドウ。権限・上限・見た目の検査と許可・拒否のログは Rust がすべて行う
  if (request.method.startsWith(ARTIFACT_BRIDGE_METHOD_SUB_WINDOW_PREFIX)) {
    try {
      const cmd = subWindowCommandFor(request.method, request.params);
      if (!cmd) throw new Error(`このアプリでは使えない API です: ${request.method}`);
      const result = await invoke<unknown>(cmd.command, cmd.args);
      reply(request, { ok: true, result });
    } catch (e) {
      reply(request, { ok: false, error: errorText(e) });
    }
    return;
  }
  // 専用 AI ターミナル。所有・権限・上限の検査と許可・拒否のログは Rust がすべて行う
  if (request.method.startsWith(ARTIFACT_BRIDGE_METHOD_AI_TERMINAL_PREFIX)) {
    try {
      const cmd = aiTerminalCommandFor(request.method, request.params);
      if (!cmd) throw new Error(`このアプリでは使えない API です: ${request.method}`);
      const result = await invoke<unknown>(cmd.command, cmd.args);
      reply(request, { ok: true, result });
    } catch (e) {
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
    applyWindowBackground(next);
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
    // 専用 AI ターミナルのフックイベント（Stop など）。Rust が `emit_to(label)` で、このウィンドウだけへ送る
    unlisteners.push(
      await getCurrentWebviewWindow().listen<Record<string, unknown>>("app-ai-terminal-event", (event) => {
        if (nonce.value) postArtifactBridgeAiTerminalEvent(frame.value, event.payload, nonce.value);
      }),
    );
    // 自分が開いたサブウィンドウがユーザーに閉じられた通知（Rust が `emit_to(label)` で、このインスタンスの
    // ウィンドウだけへ送る）
    unlisteners.push(
      await getCurrentWebviewWindow().listen<Record<string, unknown>>("app-subwindow-event", (event) => {
        if (nonce.value) postArtifactBridgeSubWindowEvent(frame.value, event.payload, nonce.value);
      }),
    );
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
    applyWindowBackground(b);
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
  <div class="appinst-shell" :class="{ transparent: boot?.subWindow?.transparent }">
    <div v-if="loadError" class="shell-error">{{ loadError }}</div>
    <iframe
      v-else-if="srcdoc"
      ref="frame"
      :srcdoc="srcdoc"
      sandbox="allow-scripts"
      class="app-frame"
      :class="{ transparent: boot?.subWindow?.transparent }"
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
/* 透明なサブウィンドウ。iframe のページ側も透明にして、アプリが塗った部分だけが見えるようにする */
.appinst-shell.transparent,
.app-frame.transparent {
  background: transparent;
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
