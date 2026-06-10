import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import type { Terminal } from "@xterm/xterm";

import { usePtyWriteBatcher } from "./usePtyWriteBatcher";

// vitest 環境は node のため requestAnimationFrame 等は未定義。
// テストごとに stub し、「rAF を発火させない=オクルージョン」状況を再現する。

const FALLBACK_FLUSH_MS = 200;
const HIGH_WATERMARK_BYTES = 8 * 1024 * 1024;

function makeTerminal(): { terminal: Terminal; write: ReturnType<typeof vi.fn> } {
  const write = vi.fn();
  // 必要なメソッドのみのスタブ。型は Terminal として扱う。
  const terminal = { write } as unknown as Terminal;
  return { terminal, write };
}

/** rAF を「id を返すが never fire(オクルージョン)」にする。 */
function stubRafNeverFire() {
  let nextId = 1;
  vi.stubGlobal(
    "requestAnimationFrame",
    vi.fn(() => nextId++)
  );
  vi.stubGlobal("cancelAnimationFrame", vi.fn());
}

/** rAF を「登録時に即座に同期発火(前面)」にする。 */
function stubRafImmediate() {
  vi.stubGlobal(
    "requestAnimationFrame",
    vi.fn((cb: FrameRequestCallback) => {
      cb(0);
      return 1;
    })
  );
  vi.stubGlobal("cancelAnimationFrame", vi.fn());
}

afterEach(() => {
  vi.unstubAllGlobals();
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe("usePtyWriteBatcher", () => {
  describe("ケースA: フォールバック排出 (rAF停止・setTimeout が排出)", () => {
    beforeEach(() => {
      vi.useFakeTimers();
      stubRafNeverFire();
    });

    it("rAF が発火しなくても FALLBACK_FLUSH_MS 後に 1回マージして write される", () => {
      const { terminal, write } = makeTerminal();
      const { enqueue } = usePtyWriteBatcher(() => terminal);

      enqueue(new Uint8Array([1, 2]));
      enqueue(new Uint8Array([3]));
      enqueue(new Uint8Array([4, 5]));

      // rAF は never fire。タイマ未経過では write されない。
      expect(write).not.toHaveBeenCalled();

      vi.advanceTimersByTime(FALLBACK_FLUSH_MS);

      expect(write).toHaveBeenCalledTimes(1);
      expect(write.mock.calls[0][0]).toEqual(new Uint8Array([1, 2, 3, 4, 5]));
    });

    it("flush 後に新たな enqueue があれば次のタイマで再度排出される", () => {
      const { terminal, write } = makeTerminal();
      const { enqueue } = usePtyWriteBatcher(() => terminal);

      enqueue(new Uint8Array([1]));
      vi.advanceTimersByTime(FALLBACK_FLUSH_MS);
      expect(write).toHaveBeenCalledTimes(1);

      enqueue(new Uint8Array([2]));
      vi.advanceTimersByTime(FALLBACK_FLUSH_MS);
      expect(write).toHaveBeenCalledTimes(2);
      expect(write.mock.calls[1][0]).toEqual(new Uint8Array([2]));
    });
  });

  describe("ケースB: 上限即時排出 (HIGH_WATERMARK 超過)", () => {
    beforeEach(() => {
      vi.useFakeTimers();
      stubRafNeverFire();
    });

    it("合計が HIGH_WATERMARK_BYTES を超えたら、タイマを進めずに即時 write される", () => {
      const { terminal, write } = makeTerminal();
      const { enqueue } = usePtyWriteBatcher(() => terminal);

      const half = Math.floor(HIGH_WATERMARK_BYTES / 2) + 1; // 2回で上限超過
      enqueue(new Uint8Array(half));
      expect(write).not.toHaveBeenCalled(); // 1回目は上限未満

      enqueue(new Uint8Array(half)); // ここで上限超過 → 同期 flush

      // タイマを一切進めていないことが要点(throttle 非依存の上限保証)。
      expect(write).toHaveBeenCalledTimes(1);
      expect(write.mock.calls[0][0].length).toBe(half * 2);
    });

    it("上限 flush 後はバッファがリセットされ、再蓄積が 0 から始まる", () => {
      const { terminal, write } = makeTerminal();
      const { enqueue } = usePtyWriteBatcher(() => terminal);

      enqueue(new Uint8Array(HIGH_WATERMARK_BYTES)); // 単発で上限到達 → 即 flush
      expect(write).toHaveBeenCalledTimes(1);

      // リセットされていれば、小チャンクは上限に達せずタイマ排出になる。
      enqueue(new Uint8Array([9]));
      expect(write).toHaveBeenCalledTimes(1);
      vi.advanceTimersByTime(FALLBACK_FLUSH_MS);
      expect(write).toHaveBeenCalledTimes(2);
      expect(write.mock.calls[1][0]).toEqual(new Uint8Array([9]));
    });
  });

  describe("ケースC: 前面・回帰 (rAF 即時発火)", () => {
    beforeEach(() => {
      stubRafImmediate();
    });

    it("rAF が即発火する環境では 1 enqueue ごとに従来どおり write される", () => {
      const { terminal, write } = makeTerminal();
      const { enqueue } = usePtyWriteBatcher(() => terminal);

      enqueue(new Uint8Array([1, 2, 3]));
      expect(write).toHaveBeenCalledTimes(1);
      expect(write.mock.calls[0][0]).toEqual(new Uint8Array([1, 2, 3]));
    });
  });

  describe("dispose", () => {
    beforeEach(() => {
      vi.useFakeTimers();
      stubRafNeverFire();
    });

    it("保留中チャンクを flush し、スケジュールを解除する", () => {
      const { terminal, write } = makeTerminal();
      const { enqueue, dispose } = usePtyWriteBatcher(() => terminal);

      enqueue(new Uint8Array([7, 8]));
      dispose();

      expect(write).toHaveBeenCalledTimes(1);
      expect(write.mock.calls[0][0]).toEqual(new Uint8Array([7, 8]));

      // dispose 後にタイマが進んでも追加 write は起きない(スケジュール解除済み)。
      vi.advanceTimersByTime(FALLBACK_FLUSH_MS * 2);
      expect(write).toHaveBeenCalledTimes(1);
    });
  });
});
