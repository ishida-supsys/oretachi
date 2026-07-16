import { listen } from "@tauri-apps/api/event";
import { decodePtyOutput } from "../utils/decodePtyOutput";
import { logDebug } from "../utils/log";

/**
 * ハンドラ未登録セッションの保留バッファ上限 (per-session)。
 * Rust 側 `pty_manager.rs` の `MAX_PENDING_BYTES`(8MB) と揃える。
 * 超過時は最古チャンクから破棄する (無制限蓄積によるヒープ肥大の防止)。
 */
const MAX_PENDING_BUFFER_BYTES = 8 * 1024 * 1024;

interface PtyOutputPayload {
  sessionId: number;
  data: string;
}

interface PtyExitPayload {
  sessionId: number;
}

type OutputHandler = (data: Uint8Array) => void;
type ExitHandler = () => void;

const outputHandlers = new Map<number, OutputHandler>();
const exitHandlers = new Map<number, ExitHandler>();
const dirtySessionIds = new Set<number>();
const pendingBuffers = new Map<number, Uint8Array[]>();
const pendingBufferBytes = new Map<number, number>();
let initialized = false;

async function init() {
  if (initialized) return;
  initialized = true;

  await listen<PtyOutputPayload>("pty-output", (event) => {
    const { sessionId, data } = event.payload;
    dirtySessionIds.add(sessionId);
    const bytes = decodePtyOutput(data);
    const handler = outputHandlers.get(sessionId);
    if (handler) {
      handler(bytes);
    } else {
      let buf = pendingBuffers.get(sessionId);
      if (!buf) {
        buf = [];
        pendingBuffers.set(sessionId, buf);
      }
      buf.push(bytes);
      let total = (pendingBufferBytes.get(sessionId) ?? 0) + bytes.length;
      // 上限超過時は最古チャンクから破棄する
      let droppedBytes = 0;
      while (total > MAX_PENDING_BUFFER_BYTES && buf.length > 1) {
        const dropped = buf.shift()!;
        total -= dropped.length;
        droppedBytes += dropped.length;
      }
      if (droppedBytes > 0) {
        logDebug(
          `[PtyDispatcher] pending buffer overflow sid=${sessionId} dropped=${droppedBytes}B kept=${total}B`
        );
      }
      pendingBufferBytes.set(sessionId, total);
    }
  });

  await listen<PtyExitPayload>("pty-exit", (event) => {
    const { sessionId } = event.payload;
    exitHandlers.get(sessionId)?.();
  });
}

// 初期化をモジュールロード時に開始
const initPromise = init();

/** sessionIdを確定してハンドラ登録し、バッファに溜まった出力をフラッシュする。 */
export function activateSession(
  sessionId: number,
  onOutput: OutputHandler,
  onExit: ExitHandler
): void {
  outputHandlers.set(sessionId, onOutput);
  exitHandlers.set(sessionId, onExit);
  const buf = pendingBuffers.get(sessionId);
  if (buf) {
    pendingBuffers.delete(sessionId);
    pendingBufferBytes.delete(sessionId);
    for (const data of buf) {
      onOutput(data);
    }
  }
}

export function registerPtyHandlers(
  sessionId: number,
  onOutput: OutputHandler,
  onExit: ExitHandler
): void {
  outputHandlers.set(sessionId, onOutput);
  exitHandlers.set(sessionId, onExit);
}

export function unregisterPtyHandlers(sessionId: number): void {
  outputHandlers.delete(sessionId);
  exitHandlers.delete(sessionId);
  dirtySessionIds.delete(sessionId);
  pendingBuffers.delete(sessionId);
  pendingBufferBytes.delete(sessionId);
}

export function isDirty(sessionId: number): boolean {
  return dirtySessionIds.has(sessionId);
}

export function clearDirty(sessionId: number): void {
  dirtySessionIds.delete(sessionId);
}

export function waitForInit(): Promise<void> {
  return initPromise;
}
