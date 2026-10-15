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
  ARTIFACT_APP_FLAG,
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
  options?: { standalone?: boolean; app?: boolean },
) {
  const posted: PostedRequest[] = [];
  let onMessage: ((event: { data: unknown }) => void) | null = null;

  const fakeWindow: Record<string, unknown> = {
    [ARTIFACT_STANDALONE_FLAG]: options?.standalone === true,
    [ARTIFACT_APP_FLAG]: options?.app === true,
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

// アプリ（常駐ウィンドウ）だけに効く: 保存の最大待ちと、拒否された書き込みの巻き戻し（#387）
describe("ARTIFACT_BRIDGE_JS: app の最大待ちと巻き戻し", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  const ok = { ok: true };
  const rejectedByHost = { ok: false, error: "メモリーがこのアプリの承認済み最大サイズを超えています" };

  it("100ms 間隔で書き続けても、最大待ち内に保存され Promise が解決する", async () => {
    const { bridge, posted, reply } = setupBridge({}, { app: true });
    const first = bridge.setMemoryKey("c", 0);
    for (let n = 1; n < 10; n++) {
      await vi.advanceTimersByTimeAsync(100);
      void bridge.setMemoryKey("c", n);
    }
    // 900ms 経過。debounce（400ms）は毎回延びて明けていないが、1000ms で必ず送る
    expect(posted).toHaveLength(0);
    await vi.advanceTimersByTimeAsync(100);
    expect(posted).toHaveLength(1);
    expect(posted[0].params.memory).toEqual({ c: 9 });
    reply(posted[0].requestId, ok);
    await expect(first).resolves.toBeUndefined();
  });

  it("書き続ける間、最大待ちごとに保存が続く", async () => {
    const { bridge, posted, reply } = setupBridge({}, { app: true });
    const answered = new Set<string>();
    for (let n = 0; n < 30; n++) {
      void bridge.setMemoryKey("c", n);
      await vi.advanceTimersByTimeAsync(100);
      for (const req of posted) {
        if (!answered.has(req.requestId)) {
          answered.add(req.requestId);
          reply(req.requestId, ok);
        }
      }
      await vi.advanceTimersByTimeAsync(0);
    }
    expect(posted.length).toBeGreaterThanOrEqual(3);
  });

  it("通常のアーティファクト（app でない）は書き続ける間は保存しない（挙動不変）", async () => {
    const { bridge, posted } = setupBridge({});
    for (let n = 0; n < 20; n++) {
      void bridge.setMemoryKey("c", n);
      await vi.advanceTimersByTimeAsync(100);
    }
    expect(posted).toHaveLength(0);
  });

  it("止まれば従来どおり debounce で保存し、最大待ちで二重に送らない", async () => {
    const { bridge, posted, reply } = setupBridge({}, { app: true });
    const p = bridge.setMemoryKey("a", 1);
    await vi.advanceTimersByTimeAsync(400);
    expect(posted).toHaveLength(1);
    reply(posted[0].requestId, ok);
    await p;
    await vi.advanceTimersByTimeAsync(5000);
    expect(posted).toHaveLength(1);
  });

  it("host に拒否されたら reject し、拒否された値を state から外して通知する", async () => {
    const { bridge, posted, reply } = setupBridge({ keep: 1 }, { app: true });
    const seen: Record<string, unknown>[] = [];
    bridge.subscribeMemory((m) => seen.push(m));
    const p = bridge.setMemoryKey("big", "x");
    const rejected = expect(p).rejects.toThrow("最大サイズ");
    await vi.advanceTimersByTimeAsync(400);
    reply(posted[0].requestId, rejectedByHost);
    await rejected;
    expect(bridge.getMemory()).toEqual({ keep: 1 });
    expect(seen[seen.length - 1]).toEqual({ keep: 1 });
  });

  it("拒否のあと、無関係なキーの書き込みは通り、拒否された値は再送されない", async () => {
    const { bridge, posted, reply } = setupBridge({}, { app: true });
    const p1 = bridge.setMemoryKey("big", "x");
    const r1 = expect(p1).rejects.toThrow();
    await vi.advanceTimersByTimeAsync(400);
    reply(posted[0].requestId, rejectedByHost);
    await r1;

    for (const key of ["a", "b", "c"]) {
      const p = bridge.setMemoryKey(key, 1);
      await vi.advanceTimersByTimeAsync(400);
      const req = posted[posted.length - 1];
      expect("big" in (req.params.memory as object)).toBe(false);
      reply(req.requestId, ok);
      await expect(p).resolves.toBeUndefined();
    }
    // 拒否された memory.set は最初の 1 回だけ（以後は全部通った）
    expect(posted).toHaveLength(4);
  });

  it("同じ debounce 窓に入った無関係なキーも、同じ batch として reject・巻き戻しになる", async () => {
    const { bridge, posted, reply } = setupBridge({}, { app: true });
    const p1 = bridge.setMemoryKey("big", "x");
    const p2 = bridge.setMemoryKey("other", 1);
    const r1 = expect(p1).rejects.toThrow();
    const r2 = expect(p2).rejects.toThrow();
    await vi.advanceTimersByTimeAsync(400);
    reply(posted[0].requestId, rejectedByHost);
    await r1;
    await r2;
    expect(bridge.getMemory()).toEqual({});
  });

  it("送信中に別キーへ書いた値は、巻き戻しで消えない", async () => {
    const { bridge, posted, reply } = setupBridge({}, { app: true });
    const p1 = bridge.setMemoryKey("big", "x");
    const r1 = expect(p1).rejects.toThrow();
    await vi.advanceTimersByTimeAsync(400);
    void bridge.setMemoryKey("later", 2);
    reply(posted[0].requestId, rejectedByHost);
    await r1;
    expect(bridge.getMemory()).toEqual({ later: 2 });
    await vi.advanceTimersByTimeAsync(400);
    expect(posted[1].params.memory).toEqual({ later: 2 });
  });

  it("送信中に同じキーへ書き直した値は、巻き戻しで消えない", async () => {
    const { bridge, posted, reply } = setupBridge({ k: "old" }, { app: true });
    const p1 = bridge.setMemoryKey("k", "huge");
    const r1 = expect(p1).rejects.toThrow();
    await vi.advanceTimersByTimeAsync(400);
    void bridge.setMemoryKey("k", "small");
    reply(posted[0].requestId, rejectedByHost);
    await r1;
    expect(bridge.getMemory()).toEqual({ k: "small" });
  });

  it("setMemory でキーを削除した書き込みが拒否されたら、そのキーを復元する", async () => {
    const { bridge, posted, reply } = setupBridge({ a: 1, b: 2 }, { app: true });
    const p = bridge.setMemory({ a: 1 });
    const r = expect(p).rejects.toThrow();
    await vi.advanceTimersByTimeAsync(400);
    reply(posted[0].requestId, rejectedByHost);
    await r;
    expect(bridge.getMemory()).toEqual({ a: 1, b: 2 });
  });

  it("state を直接書き換えるアプリで、送信中に書いた別キーは拒否の巻き戻しで消えない", async () => {
    const { bridge, posted, reply } = setupBridge({ a: 1 }, { app: true });
    const p = bridge.setMemory((s: Record<string, unknown>) => {
      s.big = "x";
      return s;
    });
    const r = expect(p).rejects.toThrow();
    await vi.advanceTimersByTimeAsync(400);
    // 送信中（返信前）に同じ state オブジェクトへ別キーを直接書く
    bridge.getMemory().other = 2;
    reply(posted[0].requestId, rejectedByHost);
    await r;
    expect(bridge.getMemory()).toEqual({ a: 1, other: 2 });
  });

  it("setMemory で state を直接書き換えて返すアプリでも、拒否で巻き戻る", async () => {
    const { bridge, posted, reply } = setupBridge({ a: 1 }, { app: true });
    const p = bridge.setMemory((s: Record<string, unknown>) => {
      s.big = "x";
      return s;
    });
    const r = expect(p).rejects.toThrow();
    await vi.advanceTimersByTimeAsync(400);
    reply(posted[0].requestId, rejectedByHost);
    await r;
    expect(bridge.getMemory()).toEqual({ a: 1 });
  });

  it("成功した保存は次の拒否の巻き戻し先になる", async () => {
    const { bridge, posted, reply } = setupBridge({}, { app: true });
    const p1 = bridge.setMemoryKey("a", 1);
    await vi.advanceTimersByTimeAsync(400);
    reply(posted[0].requestId, ok);
    await p1;
    const p2 = bridge.setMemoryKey("a", 2);
    const r2 = expect(p2).rejects.toThrow();
    await vi.advanceTimersByTimeAsync(400);
    reply(posted[1].requestId, rejectedByHost);
    await r2;
    expect(bridge.getMemory()).toEqual({ a: 1 });
  });

  it("送信中に外から差し替わったあとの拒否は巻き戻さない（外の書き込みを消さない）", async () => {
    const { bridge, posted, reply, push } = setupBridge({ n: 1 }, { app: true });
    const p = bridge.setMemoryKey("n", 5);
    const r = expect(p).rejects.toThrow();
    await vi.advanceTimersByTimeAsync(400);
    // 外から n=5 が書かれた（値は送信中の値と偶然同じ）
    push({
      [ARTIFACT_BRIDGE_PUSH_MARKER]: true,
      event: ARTIFACT_BRIDGE_PUSH_MEMORY_CHANGED,
      memory: { n: 5, fromMcp: true },
    });
    reply(posted[0].requestId, rejectedByHost);
    await r;
    expect(bridge.getMemory()).toEqual({ n: 5, fromMcp: true });
  });

  it("タイムアウト（拒否ではない）では巻き戻さない", async () => {
    const { bridge, posted } = setupBridge({}, { app: true });
    const p = bridge.setMemoryKey("a", 1);
    const r = expect(p).rejects.toThrow("oretachi bridge timeout");
    await vi.advanceTimersByTimeAsync(400);
    expect(posted).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(10000);
    await r;
    expect(bridge.getMemory()).toEqual({ a: 1 });
  });

  it("iframe 側の上限超過判定でも巻き戻す（無関係なキーが以後通る）", async () => {
    const { bridge, posted, reply } = setupBridge({}, { app: true });
    const p1 = bridge.setMemoryKey("big", "x".repeat(ARTIFACT_MEMORY_MAX_BYTES));
    const r1 = expect(p1).rejects.toThrow("too large");
    await vi.advanceTimersByTimeAsync(400);
    await r1;
    expect(bridge.getMemory()).toEqual({});
    const p2 = bridge.setMemoryKey("a", 1);
    await vi.advanceTimersByTimeAsync(400);
    reply(posted[0].requestId, ok);
    await expect(p2).resolves.toBeUndefined();
  });

  it("通常のアーティファクトは、拒否されても巻き戻さない（従来どおり）", async () => {
    const { bridge, posted, reply } = setupBridge({});
    const p = bridge.setMemoryKey("big", "x");
    const r = expect(p).rejects.toThrow();
    await vi.advanceTimersByTimeAsync(400);
    reply(posted[0].requestId, rejectedByHost);
    await r;
    expect(bridge.getMemory()).toEqual({ big: "x" });
  });
});
