import { ref, onUnmounted } from "vue";
import { invoke } from "@tauri-apps/api/core";
import {
  activateSession,
  registerPtyHandlers,
  unregisterPtyHandlers,
  waitForInit,
} from "./usePtyDispatcher";

export function usePty() {
  const sessionId = ref<number | null>(null);
  const isRunning = ref(false);
  const detached = ref(false);

  async function spawn(
    rows: number,
    cols: number,
    onOutput: (data: Uint8Array) => void,
    onExit: () => void,
    shell?: string,
    cwd?: string
  ): Promise<void> {
    // 既存セッションをクリーンアップ
    await cleanup();

    await waitForInit();

    const id = await invoke<number>("pty_spawn", {
      rows,
      cols,
      shell: shell ?? null,
      cwd: cwd ?? null,
    });

    sessionId.value = id;
    isRunning.value = true;

    // sessionIdを確定してハンドラ登録＆バッファフラッシュ
    activateSession(id, onOutput, () => {
      isRunning.value = false;
      onExit();
    });
  }

  async function write(data: string | Uint8Array): Promise<void> {
    if (sessionId.value === null) return;

    let bytes: number[];
    if (typeof data === "string") {
      bytes = Array.from(new TextEncoder().encode(data));
    } else {
      bytes = Array.from(data);
    }

    await invoke("pty_write", {
      sessionId: sessionId.value,
      data: bytes,
    });
  }

  // pty_resize は async コマンド (spawn_blocking) のため、並行 invoke では適用順が
  // 逆転しうる。直列化して「最後に要求したサイズが最後に適用される」ことを保証する。
  let resizeChain: Promise<void> = Promise.resolve();

  async function resize(rows: number, cols: number): Promise<void> {
    if (sessionId.value === null) return;
    const next = resizeChain.then(async () => {
      if (sessionId.value === null) return;
      await invoke("pty_resize", {
        sessionId: sessionId.value,
        rows,
        cols,
      });
    });
    resizeChain = next.catch(() => {});
    return next;
  }

  // kill の invoke 解決前に onUnmounted が再入すると sessionId がまだ非 null のため
  // pty_kill が二重発行される (ログ上 2 連発が観測されている)。in-flight の Promise を
  // 共有することで二重発行を抑止しつつ、再入側も kill 完了を待てるようにする。
  let killPromise: Promise<void> | null = null;

  function kill(): Promise<void> {
    if (sessionId.value === null) return Promise.resolve();
    if (killPromise) return killPromise;
    killPromise = (async () => {
      try {
        await invoke("pty_kill", { sessionId: sessionId.value });
        await cleanup();
      } finally {
        killPromise = null;
      }
    })();
    return killPromise;
  }

  async function attachToSession(
    id: number,
    onOutput: (data: Uint8Array) => void,
    onExit: () => void
  ): Promise<void> {
    if (sessionId.value !== null) {
      unregisterPtyHandlers(sessionId.value);
    }

    sessionId.value = id;
    isRunning.value = true;

    await waitForInit();

    registerPtyHandlers(
      id,
      onOutput,
      () => {
        isRunning.value = false;
        onExit();
      }
    );
  }

  async function cleanup(): Promise<void> {
    if (sessionId.value !== null) {
      unregisterPtyHandlers(sessionId.value);
    }
    sessionId.value = null;
    isRunning.value = false;
  }

  function detach(): void {
    // イベントハンドラを解除するが、PTY プロセスは kill しない
    if (sessionId.value !== null) {
      unregisterPtyHandlers(sessionId.value);
    }
    detached.value = true;
  }

  onUnmounted(() => {
    if (!detached.value) {
      // fire-and-forget: 失敗は unhandled rejection にせずログに残す
      kill().catch((e) => {
        console.error("[Terminal] kill on unmount failed:", e);
      });
    }
  });

  return {
    sessionId,
    isRunning,
    spawn,
    attachToSession,
    write,
    resize,
    kill,
    detach,
  };
}
