/**
 * iframe 内で動くブリッジ本体（`ARTIFACT_BRIDGE_JS`）の振る舞いを、
 * window / document / parent を差し替えて node 上で動かして確認する。
 *
 * 実体は srcdoc へ埋め込む文字列なので、`new Function` で必要なグローバルだけ束縛して
 * 評価する（`document.getElementById('_memory')` と `parent.postMessage` しか触らない）。
 */
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import {
  ARTIFACT_BRIDGE_JS,
  ARTIFACT_MEMORY_MAX_BYTES,
  ARTIFACT_BRIDGE_REQUEST_MARKER,
  ARTIFACT_BRIDGE_RESULT_MARKER,
  ARTIFACT_BRIDGE_METHOD_MEMORY_SET,
} from "./artifactMemory";

interface PostedRequest {
  requestId: string;
  method: string;
  params: { memory?: Record<string, unknown> };
}

interface Bridge {
  memory: Record<string, unknown>;
  getMemory(): Record<string, unknown>;
  setMemory(next: unknown): Promise<void>;
  setMemoryKey(key: string, value: unknown): Promise<void>;
  clearMemory(): Promise<void>;
  call(method: string, params?: unknown): Promise<unknown>;
}

function setupBridge(initialMemory: Record<string, unknown>) {
  const posted: PostedRequest[] = [];
  let onMessage: ((event: { data: unknown }) => void) | null = null;

  const fakeWindow: Record<string, unknown> = {
    addEventListener(type: string, fn: (event: { data: unknown }) => void) {
      if (type === "message") onMessage = fn;
    },
  };
  const fakeDocument = {
    getElementById(id: string) {
      return id === "_memory" ? { value: JSON.stringify(initialMemory) } : null;
    },
  };
  const fakeParent = {
    postMessage(data: Record<string, unknown>) {
      expect(data[ARTIFACT_BRIDGE_REQUEST_MARKER]).toBe(true);
      posted.push(data as unknown as PostedRequest);
    },
  };

  // eslint-disable-next-line no-new-func
  new Function("window", "document", "parent", ARTIFACT_BRIDGE_JS)(
    fakeWindow,
    fakeDocument,
    fakeParent,
  );

  /** 親からの応答。source を差し替えれば別 iframe の偽装を再現できる */
  const reply = (
    requestId: string,
    outcome: Record<string, unknown>,
    source: unknown = fakeParent,
  ) => {
    if (!onMessage) throw new Error("bridge did not register a message listener");
    onMessage({
      source,
      data: { [ARTIFACT_BRIDGE_RESULT_MARKER]: true, requestId, ...outcome },
    } as { data: unknown });
  };

  return { bridge: fakeWindow.__oretachi as Bridge, posted, reply };
}

describe("ARTIFACT_BRIDGE_JS", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("初期値を _memory から同期で読む", () => {
    const { bridge } = setupBridge({ name: "taro" });
    expect(bridge.getMemory()).toEqual({ name: "taro" });
    // memory はゲッターなので、差し替え後も最新を指す
    expect(bridge.memory).toEqual({ name: "taro" });
  });

  it("debounce 中の連続更新は 1 回の保存にまとめる", async () => {
    const { bridge, posted, reply } = setupBridge({});
    const p1 = bridge.setMemoryKey("a", 1);
    const p2 = bridge.setMemoryKey("b", 2);
    expect(posted).toHaveLength(0);

    await vi.advanceTimersByTimeAsync(400);
    expect(posted).toHaveLength(1);
    expect(posted[0].method).toBe(ARTIFACT_BRIDGE_METHOD_MEMORY_SET);
    expect(posted[0].params.memory).toEqual({ a: 1, b: 2 });

    reply(posted[0].requestId, { ok: true });
    await expect(p1).resolves.toBeUndefined();
    await expect(p2).resolves.toBeUndefined();
  });

  it("保存は 1 本ずつ。応答前の更新は前の保存が返ってから送る", async () => {
    const { bridge, posted, reply } = setupBridge({});
    void bridge.setMemoryKey("a", 1);
    await vi.advanceTimersByTimeAsync(400);
    expect(posted).toHaveLength(1);

    // まだ応答していないので、次の debounce が明けても 2 本目は飛ばない
    const p2 = bridge.setMemoryKey("b", 2);
    await vi.advanceTimersByTimeAsync(2000);
    expect(posted).toHaveLength(1);

    // 1 本目が返ると、その間に積まれた分をまとめて送る
    reply(posted[0].requestId, { ok: true });
    await vi.advanceTimersByTimeAsync(0);
    expect(posted).toHaveLength(2);
    expect(posted[1].params.memory).toEqual({ a: 1, b: 2 });

    reply(posted[1].requestId, { ok: true });
    await expect(p2).resolves.toBeUndefined();
  });

  it("保存の失敗は setMemory の Promise へ伝わる", async () => {
    const { bridge, posted, reply } = setupBridge({});
    const p = bridge.setMemory({ big: "x" });
    // reject より先にハンドラを付ける（付ける前に落ちると unhandled rejection になる）
    const rejected = expect(p).rejects.toThrow("メモリーが上限を超えています");
    await vi.advanceTimersByTimeAsync(400);
    reply(posted[0].requestId, { ok: false, error: "メモリーが上限を超えています" });
    await rejected;
  });

  it("応答が返らなければタイムアウトで reject する", async () => {
    const { bridge, posted } = setupBridge({});
    const p = bridge.setMemoryKey("a", 1);
    const rejected = expect(p).rejects.toThrow("oretachi bridge timeout");
    await vi.advanceTimersByTimeAsync(400);
    expect(posted).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(10000);
    await rejected;
  });

  it("オブジェクト以外の setMemory は同期で投げる", () => {
    const { bridge } = setupBridge({});
    expect(() => bridge.setMemory("x")).toThrow("plain object");
    expect(() => bridge.setMemory([1, 2])).toThrow("plain object");
  });

  it("clearMemory は空オブジェクトを保存する", async () => {
    const { bridge, posted, reply } = setupBridge({ a: 1 });
    const p = bridge.clearMemory();
    await vi.advanceTimersByTimeAsync(400);
    expect(posted[0].params.memory).toEqual({});
    expect(bridge.getMemory()).toEqual({});
    reply(posted[0].requestId, { ok: true });
    await expect(p).resolves.toBeUndefined();
  });

  it("未知の requestId の応答は無視する（別 iframe の取り違え防止）", async () => {
    const { bridge, posted, reply } = setupBridge({});
    const p = bridge.setMemoryKey("a", 1);
    await vi.advanceTimersByTimeAsync(400);
    reply("someone-else", { ok: true });
    reply(posted[0].requestId, { ok: true });
    await expect(p).resolves.toBeUndefined();
  });

  it("親以外から来た応答は無視する", async () => {
    const { bridge, posted, reply } = setupBridge({});
    const p = bridge.setMemoryKey("a", 1);
    await vi.advanceTimersByTimeAsync(400);
    // 同一ウィンドウ内の別 iframe が requestId を当てて成功を偽装しても通らない
    reply(posted[0].requestId, { ok: true }, { notParent: true });
    reply(posted[0].requestId, { ok: true });
    await expect(p).resolves.toBeUndefined();
  });

  it("上限超過は IPC を往復させず iframe 側で reject する", async () => {
    const { bridge, posted } = setupBridge({});
    const p = bridge.setMemoryKey("big", "x".repeat(ARTIFACT_MEMORY_MAX_BYTES));
    const rejected = expect(p).rejects.toThrow("too large");
    await vi.advanceTimersByTimeAsync(400);
    expect(posted).toHaveLength(0);
    await rejected;
  });
});
