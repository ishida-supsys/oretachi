import type { Terminal } from "@xterm/xterm";

/**
 * PTY 出力データを requestAnimationFrame ベースでバッチ化し、
 * 1フレームに1回だけ terminal.write() を呼ぶ。
 */
export function usePtyWriteBatcher(getTerminal: () => Terminal | null) {
  let chunks: Uint8Array[] = [];
  let totalLength = 0;
  let rafId: number | null = null;

  function enqueue(data: Uint8Array): void {
    chunks.push(data);
    totalLength += data.length;
    if (rafId === null) {
      rafId = requestAnimationFrame(flush);
    }
  }

  function flush(): void {
    rafId = null;
    const terminal = getTerminal();
    if (!terminal || chunks.length === 0) {
      chunks = [];
      totalLength = 0;
      return;
    }

    if (chunks.length === 1) {
      terminal.write(chunks[0]);
    } else {
      const merged = new Uint8Array(totalLength);
      let offset = 0;
      for (const chunk of chunks) {
        merged.set(chunk, offset);
        offset += chunk.length;
      }
      terminal.write(merged);
    }

    chunks = [];
    totalLength = 0;
  }

  function dispose(): void {
    if (rafId !== null) {
      cancelAnimationFrame(rafId);
      rafId = null;
    }
    flush();
  }

  return { enqueue, dispose };
}
