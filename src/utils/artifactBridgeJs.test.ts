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
  ARTIFACT_BRIDGE_METHOD_MCP_CALL,
  ARTIFACT_BRIDGE_PUSH_MARKER,
  ARTIFACT_BRIDGE_PUSH_MEMORY_CHANGED,
  ARTIFACT_STANDALONE_FLAG,
} from "./artifactMemory";

interface PostedRequest {
  requestId: string;
  method: string;
  params: { memory?: Record<string, unknown>; tool?: string; params?: Record<string, unknown> };
}

interface Bridge {
  memory: Record<string, unknown>;
  getMemory(): Record<string, unknown>;
  setMemory(next: unknown): Promise<void>;
  setMemoryKey(key: string, value: unknown): Promise<void>;
  clearMemory(): Promise<void>;
  call(method: string, params?: unknown): Promise<unknown>;
  callTool(tool: unknown, params?: unknown): Promise<unknown>;
  subscribeMemory(fn: (memory: Record<string, unknown>) => void): () => void;
}

function setupBridge(
  initialMemory: Record<string, unknown>,
  options?: { standalone?: boolean },
) {
  const posted: PostedRequest[] = [];
  let onMessage: ((event: { data: unknown }) => void) | null = null;

  const fakeWindow: Record<string, unknown> = {
    [ARTIFACT_STANDALONE_FLAG]: options?.standalone === true,
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

  /** 親からの一方向通知（MCP の artifact_store がストアを書き換えた場合） */
  const push = (data: Record<string, unknown>, source: unknown = fakeParent) => {
    if (!onMessage) throw new Error("bridge did not register a message listener");
    onMessage({ source, data } as { data: unknown });
  };

  return { bridge: fakeWindow.__oretachi as Bridge, posted, reply, push };
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

  it("callTool は mcp.call を送り、JSON の応答をパースして返す", async () => {
    const { bridge, posted, reply } = setupBridge({});
    const p = bridge.callTool("oretachi_write_terminal", { session_id: 12, text: "echo hi" });
    expect(posted).toHaveLength(1);
    expect(posted[0].method).toBe(ARTIFACT_BRIDGE_METHOD_MCP_CALL);
    expect(posted[0].params.tool).toBe("oretachi_write_terminal");
    expect(posted[0].params.params).toEqual({ session_id: 12, text: "echo hi" });

    reply(posted[0].requestId, { ok: true, result: JSON.stringify({ cursor: 3 }) });
    await expect(p).resolves.toEqual({ cursor: 3 });
  });

  it("callTool の応答が JSON でなければ文字列のまま返す", async () => {
    const { bridge, posted, reply } = setupBridge({});
    const p = bridge.callTool("oretachi_write_terminal", { session_id: 12, text: "x" });
    reply(posted[0].requestId, { ok: true, result: "written" });
    await expect(p).resolves.toBe("written");
  });

  it("callTool は params 省略でも空オブジェクトを送る", async () => {
    const { bridge, posted, reply } = setupBridge({});
    const p = bridge.callTool("oretachi_list_worktree_notifications");
    expect(posted[0].params.params).toEqual({});
    reply(posted[0].requestId, { ok: true, result: "[]" });
    await expect(p).resolves.toEqual([]);
  });

  it("callTool のツール名が不正なら往復させずに reject する", async () => {
    const { bridge, posted } = setupBridge({});
    await expect(bridge.callTool("")).rejects.toThrow("callTool expects a tool name");
    await expect(bridge.callTool(undefined)).rejects.toThrow("callTool expects a tool name");
    expect(posted).toHaveLength(0);
  });

  it("ホワイトリスト外などのエラーは callTool の Promise へ伝わる", async () => {
    const { bridge, posted, reply } = setupBridge({});
    const p = bridge.callTool("oretachi_kill_terminal", {});
    reply(posted[0].requestId, { ok: false, error: "ツール 'oretachi_kill_terminal' は..." });
    await expect(p).rejects.toThrow("oretachi_kill_terminal");
  });

  it("外からのストア更新を取り込んで subscribeMemory を再通知する", () => {
    const { bridge, push } = setupBridge({ answered: false });
    const seen: Record<string, unknown>[] = [];
    bridge.subscribeMemory((m) => seen.push(m));

    push({
      [ARTIFACT_BRIDGE_PUSH_MARKER]: true,
      event: ARTIFACT_BRIDGE_PUSH_MEMORY_CHANGED,
      memory: { answered: true },
    });

    expect(bridge.getMemory()).toEqual({ answered: true });
    expect(seen).toEqual([{ answered: true }]);
  });

  it("外からのストア更新を取り込んだ後の保存は、取り込んだ内容を土台にする", async () => {
    const { bridge, posted, push } = setupBridge({ a: 1 });
    push({
      [ARTIFACT_BRIDGE_PUSH_MARKER]: true,
      event: ARTIFACT_BRIDGE_PUSH_MEMORY_CHANGED,
      memory: { a: 1, answered: true },
    });

    // 押し込まれた値を土台にするので、次の 1 入力で外からの書き込みが消えない
    void bridge.setMemoryKey("b", 2);
    await vi.advanceTimersByTimeAsync(400);
    expect(posted[0].params.memory).toEqual({ a: 1, answered: true, b: 2 });
  });

  it("保存待ちの入力が外からの更新に押し流されたら reject する", async () => {
    const { bridge, posted, push } = setupBridge({ a: 1 });
    // debounce 待ちの間に外から差し替えられる
    const p = bridge.setMemoryKey("b", 2);
    push({
      [ARTIFACT_BRIDGE_PUSH_MARKER]: true,
      event: ARTIFACT_BRIDGE_PUSH_MEMORY_CHANGED,
      memory: { a: 1, answered: true },
    });

    // 押し込まれた値を送って resolve すると「保存できた」の誤報になる
    await expect(p).rejects.toThrow("replaced from outside");
    await vi.advanceTimersByTimeAsync(400);
    expect(posted).toHaveLength(0);
    expect(bridge.getMemory()).toEqual({ a: 1, answered: true });
  });

  it("親以外からの / 壊れた一方向通知は無視する", () => {
    const { bridge, push } = setupBridge({ a: 1 });
    const other = {};
    push(
      {
        [ARTIFACT_BRIDGE_PUSH_MARKER]: true,
        event: ARTIFACT_BRIDGE_PUSH_MEMORY_CHANGED,
        memory: { a: 2 },
      },
      other,
    );
    expect(bridge.getMemory()).toEqual({ a: 1 });

    // オブジェクト以外のペイロードは捨てる（state を壊さない）
    for (const memory of [null, "x", [1, 2], undefined]) {
      push({
        [ARTIFACT_BRIDGE_PUSH_MARKER]: true,
        event: ARTIFACT_BRIDGE_PUSH_MEMORY_CHANGED,
        memory,
      });
      expect(bridge.getMemory()).toEqual({ a: 1 });
    }

    // 未知の event 種別も無視する
    push({ [ARTIFACT_BRIDGE_PUSH_MARKER]: true, event: "unknown", memory: { a: 3 } });
    expect(bridge.getMemory()).toEqual({ a: 1 });
  });

  it("一方向通知は pending の応答として消費されない", async () => {
    const { bridge, posted, push, reply } = setupBridge({});
    const p = bridge.callTool("oretachi_write_terminal", { session_id: 1, text: "x" });
    push({
      [ARTIFACT_BRIDGE_PUSH_MARKER]: true,
      event: ARTIFACT_BRIDGE_PUSH_MEMORY_CHANGED,
      memory: { a: 1 },
      requestId: posted[0].requestId,
    });
    reply(posted[0].requestId, { ok: true, result: "written" });
    await expect(p).resolves.toBe("written");
  });

  // エクスポートした単体 HTML（zip の view.html）は親を持たない。
  // 保存が 10 秒ぶら下がって失敗する、という見え方にならないことを確かめる。
  describe("standalone（エクスポートした単体ファイル）", () => {
    it("メモリーはメモリ上で完結し、保存は postMessage せずに resolve する", async () => {
      const { bridge, posted } = setupBridge({ a: 1 }, { standalone: true });
      const p = bridge.setMemoryKey("b", 2);
      await vi.advanceTimersByTimeAsync(400);
      await expect(p).resolves.toBeUndefined();
      expect(posted).toHaveLength(0);
      // 保存されないだけで、表示中の値としては更新されている
      expect(bridge.getMemory()).toEqual({ a: 1, b: 2 });
    });

    it("上限超過の判定は standalone でも効く", async () => {
      const { bridge } = setupBridge({}, { standalone: true });
      const p = bridge.setMemoryKey("big", "x".repeat(ARTIFACT_MEMORY_MAX_BYTES));
      const rejected = expect(p).rejects.toThrow("too large");
      await vi.advanceTimersByTimeAsync(400);
      await rejected;
    });

    it("callTool は待たずに reject する（呼べる相手がいない）", async () => {
      const { bridge, posted } = setupBridge({}, { standalone: true });
      await expect(bridge.callTool("oretachi_write_terminal", {})).rejects.toThrow(
        "unavailable in an exported file",
      );
      expect(posted).toHaveLength(0);
    });
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
