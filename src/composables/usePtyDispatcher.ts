import { listen } from "@tauri-apps/api/event";

interface PtyOutputPayload {
  sessionId: number;
  data: number[];
}

interface PtyExitPayload {
  sessionId: number;
}

type OutputHandler = (data: Uint8Array) => void;
type ExitHandler = () => void;

const outputHandlers = new Map<number, OutputHandler>();
const exitHandlers = new Map<number, ExitHandler>();
const dirtySessionIds = new Set<number>();
let initialized = false;

async function init() {
  if (initialized) return;
  initialized = true;

  await listen<PtyOutputPayload>("pty-output", (event) => {
    const { sessionId, data } = event.payload;
    dirtySessionIds.add(sessionId);
    outputHandlers.get(sessionId)?.(new Uint8Array(data));
  });

  await listen<PtyExitPayload>("pty-exit", (event) => {
    const { sessionId } = event.payload;
    exitHandlers.get(sessionId)?.();
  });
}

// 初期化をモジュールロード時に開始
const initPromise = init();

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
