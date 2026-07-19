import type { Terminal } from "@xterm/xterm";

/** rAF が停止(オクルージョン)している場合のフォールバック排出間隔(ms)。 */
const FALLBACK_FLUSH_MS = 200;
/**
 * 蓄積バイトのハード上限。これを超えたら rAF/タイマを待たず即時 flush する。
 * Rust 側 `pty_manager.rs` の `MAX_PENDING_BYTES`(8MB)と揃える。
 */
const HIGH_WATERMARK_BYTES = 8 * 1024 * 1024;

/**
 * PTY 出力データをバッチ化し、まとめて terminal.write() を呼ぶ。
 *
 * 通常(前面表示)は requestAnimationFrame で 1フレームに 1回 flush する。
 * ただし WebView2/Chromium はウィンドウのオクルージョン(最小化・完全被覆・
 * 別仮想デスクトップ表示)時に rAF を停止する一方、`pty-output` IPC は届き続けるため、
 * rAF 一本に依存するとバッファが無制限に増大し、復帰時の巨大 write でメインスレッドが
 * 恒久ブロックする。これを防ぐため次の二段のフォールバックを併用する:
 *   1. setTimeout によるフォールバック排出(rAF が来なくても排出する)。
 *   2. HIGH_WATERMARK_BYTES 超過時の同期即時 flush(タイマ throttle に依存せず上限を保証)。
 *
 * さらに setSuspended(true) でオフスクリーン端末の排出を抑制できる。
 * 抑制中は enqueue のみ行い terminal.write() をスケジュールしない
 * (多数のオフスクリーン端末が並行して parse/render コストを消費し、
 * メインスレッドを飽和させる webview ハングの対策)。
 * HIGH_WATERMARK_BYTES 超過時は抑制中でも flush する (ANSI ストリームは破棄不可のため)。
 */
export function usePtyWriteBatcher(getTerminal: () => Terminal | null) {
  let chunks: Uint8Array[] = [];
  let totalLength = 0;
  let rafId: number | null = null;
  let timerId: ReturnType<typeof setTimeout> | null = null;
  let suspended = false;

  function clearScheduled(): void {
    if (rafId !== null) {
      cancelAnimationFrame(rafId);
      rafId = null;
    }
    if (timerId !== null) {
      clearTimeout(timerId);
      timerId = null;
    }
  }

  function enqueue(data: Uint8Array): void {
    chunks.push(data);
    totalLength += data.length;
    // 上限超過時はタイマ throttle 非依存でその場排出し、ヒープを上限内に固定する。
    if (totalLength >= HIGH_WATERMARK_BYTES) {
      flush();
      return;
    }
    // 抑制中(オフスクリーン)は蓄積のみ。setSuspended(false) 時にまとめて排出する。
    if (suspended) {
      return;
    }
    // 前面では rAF が先に発火して滑らかにバッチ。オクルージョン時は rAF が来ないため
    // setTimeout 側が排出する(両方張っておき、flush 時に両方 cancel)。
    if (rafId === null) {
      rafId = requestAnimationFrame(flush);
    }
    if (timerId === null) {
      timerId = setTimeout(flush, FALLBACK_FLUSH_MS);
    }
  }

  function flush(): void {
    // rAF/タイマの二重発火を防止(自分自身を起動した側の cancel は no-op で安全)。
    clearScheduled();
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

  /**
   * オフスクリーン時の排出抑制を切り替える。
   * 抑制解除時は蓄積分を即 flush する (可視化直後の fit/refresh が最新内容を描画できるように)。
   */
  function setSuspended(value: boolean): void {
    if (suspended === value) return;
    suspended = value;
    if (value) {
      // スケジュール済みの排出を取り消して蓄積モードへ
      clearScheduled();
    } else if (chunks.length > 0) {
      flush();
    }
  }

  function dispose(): void {
    clearScheduled();
    flush();
  }

  return { enqueue, flush, setSuspended, dispose };
}
