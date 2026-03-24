import { ref, onUnmounted } from "vue";
import { invoke } from "@tauri-apps/api/core";
import {
  reserveSession,
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

    // spawn前にハンドラを予約（spawn完了までの出力をバッファリング）
    reserveSession(onOutput, () => {
      isRunning.value = false;
      onExit();
    });

    const id = await invoke<number>("pty_spawn", {
      rows,
      cols,
      shell: shell ?? null,
      cwd: cwd ?? null,
    });

    sessionId.value = id;
    isRunning.value = true;

    // sessionIdを確定し、バッファをフラッシュ
    activateSession(id);
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

  async function resize(rows: number, cols: number): Promise<void> {
    if (sessionId.value === null) return;
    await invoke("pty_resize", {
      sessionId: sessionId.value,
      rows,
      cols,
    });
  }

  async function kill(): Promise<void> {
    if (sessionId.value === null) return;
    await invoke("pty_kill", { sessionId: sessionId.value });
    await cleanup();
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
      kill();
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
