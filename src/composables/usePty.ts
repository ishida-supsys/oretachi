import { ref, onUnmounted } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

interface PtyOutputPayload {
  sessionId: number;
  data: number[];
}

interface PtyExitPayload {
  sessionId: number;
}

export function usePty() {
  const sessionId = ref<number | null>(null);
  const isRunning = ref(false);
  const detached = ref(false);

  let unlistenOutput: UnlistenFn | null = null;
  let unlistenExit: UnlistenFn | null = null;

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

    unlistenOutput = await listen<PtyOutputPayload>("pty-output", (event) => {
      if (event.payload.sessionId !== sessionId.value) return;
      const data = new Uint8Array(event.payload.data);
      onOutput(data);
    });

    unlistenExit = await listen<PtyExitPayload>("pty-exit", (event) => {
      if (event.payload.sessionId !== sessionId.value) return;
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
    if (unlistenOutput) {
      unlistenOutput();
      unlistenOutput = null;
    }
    if (unlistenExit) {
      unlistenExit();
      unlistenExit = null;
    }

    sessionId.value = id;
    isRunning.value = true;

    unlistenOutput = await listen<PtyOutputPayload>("pty-output", (event) => {
      if (event.payload.sessionId !== sessionId.value) return;
      const data = new Uint8Array(event.payload.data);
      onOutput(data);
    });

    unlistenExit = await listen<PtyExitPayload>("pty-exit", (event) => {
      if (event.payload.sessionId !== sessionId.value) return;
      isRunning.value = false;
      onExit();
    });
  }

  async function cleanup(): Promise<void> {
    if (unlistenOutput) {
      unlistenOutput();
      unlistenOutput = null;
    }
    if (unlistenExit) {
      unlistenExit();
      unlistenExit = null;
    }
    sessionId.value = null;
    isRunning.value = false;
  }

  function detach(): void {
    // イベントリスナーを解除するが、PTY プロセスは kill しない
    if (unlistenOutput) {
      unlistenOutput();
      unlistenOutput = null;
    }
    if (unlistenExit) {
      unlistenExit();
      unlistenExit = null;
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
