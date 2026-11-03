import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import type { Terminal } from "@xterm/xterm";

interface DebugSnapshotOptions {
  getTerminal: () => Terminal | null;
  getSessionId: () => number | null;
  isOffscreen: () => boolean;
  isWebglLoaded: () => boolean;
}

/** カーソル行の前後何行を採るか（Rust 側の replay_file の window と揃える）。 */
const WINDOW = 3;

/**
 * #421 診断（dev ビルド限定）: MCP `oretachi_debug_terminal_snapshot` からの要求に、
 * xterm のバッファ・カーソル・全角セル幅・WebGL 有無を返す。
 *
 * 行番号はビューポート基準（Rust 側の参照エミュレータ vt100 と揃える）。
 * release ビルドでは `import.meta.env.DEV` が false になり何も登録しない。
 */
export function useTerminalDebugSnapshot(options: DebugSnapshotOptions) {
  let unlisten: UnlistenFn | null = null;
  let disposed = false;

  function collect() {
    const terminal = options.getTerminal();
    if (!terminal) return { error: "terminal not initialized" };
    const buf = terminal.buffer.active;
    const cy = buf.cursorY;
    const first = Math.max(0, cy - WINDOW);
    const last = Math.min(terminal.rows - 1, cy + WINDOW);
    const lines: { row: number; text: string }[] = [];
    for (let r = first; r <= last; r++) {
      lines.push({ row: r, text: buf.getLine(buf.baseY + r)?.translateToString(false) ?? "" });
    }
    const cursorRowCells: [number, string, number][] = [];
    const cursorLine = buf.getLine(buf.baseY + cy);
    if (cursorLine) {
      for (let x = 0; x < terminal.cols; x++) {
        const cell = cursorLine.getCell(x);
        if (!cell) continue;
        const chars = cell.getChars();
        const width = cell.getWidth();
        if (chars !== "" || width === 0) cursorRowCells.push([x, chars, width]);
      }
    }
    return {
      rows: terminal.rows,
      cols: terminal.cols,
      cursor: { row: cy, col: buf.cursorX },
      baseY: buf.baseY,
      viewportY: buf.viewportY,
      unicodeVersion: terminal.unicode.activeVersion,
      webglLoaded: options.isWebglLoaded(),
      offscreen: options.isOffscreen(),
      lines,
      cursorRowCells,
    };
  }

  async function start(): Promise<void> {
    if (!import.meta.env.DEV) return;
    const u = await listen<{ requestId: string; sessionId: number }>(
      "debug-terminal-snapshot-request",
      async (event) => {
        if (event.payload.sessionId !== options.getSessionId()) return;
        await invoke("mcp_debug_terminal_snapshot_result", {
          requestId: event.payload.requestId,
          snapshot: collect(),
        }).catch(() => { /* 呼び出し元がタイムアウト済みなら無視 */ });
      },
    );
    if (disposed) u();
    else unlisten = u;
  }

  function dispose(): void {
    disposed = true;
    unlisten?.();
    unlisten = null;
  }

  return { start, dispose };
}
