import { describe, it, expect, vi } from "vitest";
import { reactive, ref } from "vue";
import {
  ARTIFACT_BRIDGE_REQUEST_MARKER,
  ARTIFACT_BRIDGE_RESULT_MARKER,
  ARTIFACT_BRIDGE_METHOD_MEMORY_SET,
  ARTIFACT_BRIDGE_PUSH_MARKER,
  ARTIFACT_BRIDGE_PUSH_MEMORY_CHANGED,
  ARTIFACT_BRIDGE_PUSH_MEMORY_FLUSH,
  ARTIFACT_BRIDGE_METHOD_FLUSHED,
  ARTIFACT_BRIDGE_NONCE_GLOBAL,
  ARTIFACT_BRIDGE_JS,
  ARTIFACT_BRIDGE_METHOD_NET_FETCH,
  readArtifactBridgeRequest,
  postArtifactBridgeResult,
  postArtifactBridgeMemoryChanged,
  postArtifactBridgeFlush,
} from "./artifactMemory";
import { buildReactSrcdoc } from "./reactArtifactSrcdoc";

/** contentWindow だけを持つ iframe のスタブ */
function makeFrame(contentWindow: unknown): HTMLIFrameElement {
  return { contentWindow } as unknown as HTMLIFrameElement;
}

function makeEvent(source: unknown, data: unknown): MessageEvent {
  return { source, data } as unknown as MessageEvent;
}

describe("readArtifactBridgeRequest", () => {
  const win = {};
  const frame = makeFrame(win);
  const validData = {
    [ARTIFACT_BRIDGE_REQUEST_MARKER]: true,
    requestId: "r1",
    method: ARTIFACT_BRIDGE_METHOD_MEMORY_SET,
    params: { memory: { name: "taro" } },
  };

  it("対象 iframe からのリクエストを読む", () => {
    expect(readArtifactBridgeRequest(makeEvent(win, validData), frame)).toEqual({
      requestId: "r1",
      method: ARTIFACT_BRIDGE_METHOD_MEMORY_SET,
      params: { memory: { name: "taro" } },
    });
  });

  it("別ウィンドウからのメッセージは受け付けない", () => {
    expect(readArtifactBridgeRequest(makeEvent({}, validData), frame)).toBeNull();
  });

  it("iframe が無ければ受け付けない", () => {
    expect(readArtifactBridgeRequest(makeEvent(win, validData), null)).toBeNull();
  });

  it("マーカーが無いメッセージは無視する", () => {
    const data = { ...validData, [ARTIFACT_BRIDGE_REQUEST_MARKER]: undefined };
    expect(readArtifactBridgeRequest(makeEvent(win, data), frame)).toBeNull();
  });

  it("requestId / method が文字列でなければ無視する", () => {
    expect(
      readArtifactBridgeRequest(makeEvent(win, { ...validData, requestId: 1 }), frame),
    ).toBeNull();
    expect(
      readArtifactBridgeRequest(makeEvent(win, { ...validData, method: null }), frame),
    ).toBeNull();
  });

  it("params が無い / オブジェクトでない場合は空オブジェクトにする", () => {
    expect(
      readArtifactBridgeRequest(makeEvent(win, { ...validData, params: undefined }), frame)?.params,
    ).toEqual({});
    expect(
      readArtifactBridgeRequest(makeEvent(win, { ...validData, params: "x" }), frame)?.params,
    ).toEqual({});
  });

  it("文字列など object でない data は無視する", () => {
    expect(readArtifactBridgeRequest(makeEvent(win, "hello"), frame)).toBeNull();
    expect(readArtifactBridgeRequest(makeEvent(win, null), frame)).toBeNull();
  });
});

describe("postArtifactBridgeResult", () => {
  it("成功応答を iframe へ送る", () => {
    const postMessage = vi.fn();
    const frame = makeFrame({ postMessage });
    postArtifactBridgeResult(frame, "r1", { ok: true });
    expect(postMessage).toHaveBeenCalledWith(
      { [ARTIFACT_BRIDGE_RESULT_MARKER]: true, requestId: "r1", ok: true },
      "*",
    );
  });

  it("失敗応答はエラーメッセージを載せる", () => {
    const postMessage = vi.fn();
    const frame = makeFrame({ postMessage });
    postArtifactBridgeResult(frame, "r2", { ok: false, error: "too large" });
    expect(postMessage).toHaveBeenCalledWith(
      { [ARTIFACT_BRIDGE_RESULT_MARKER]: true, requestId: "r2", ok: false, error: "too large" },
      "*",
    );
  });

  it("iframe が消えていても例外を投げない", () => {
    expect(() => postArtifactBridgeResult(null, "r3", { ok: true })).not.toThrow();
    expect(() =>
      postArtifactBridgeResult(makeFrame(null), "r3", { ok: true }),
    ).not.toThrow();
  });
});

describe("postArtifactBridgeMemoryChanged", () => {
  it("ストアの差し替えを一方向通知として送る", () => {
    const postMessage = vi.fn();
    postArtifactBridgeMemoryChanged(makeFrame({ postMessage }), { answered: true });
    expect(postMessage).toHaveBeenCalledWith(
      {
        [ARTIFACT_BRIDGE_PUSH_MARKER]: true,
        event: ARTIFACT_BRIDGE_PUSH_MEMORY_CHANGED,
        memory: { answered: true },
      },
      "*",
    );
  });

  /**
   * 呼び出し側が渡してくるのは Vue の `states` ref 由来の値で、reactive Proxy を
   * そのまま postMessage に渡すと構造化複製が DataCloneError で落ちる。
   * 落ちると外からの更新が iframe に届かず、次の 1 入力で消える（しかも MCP 側は
   * 成功を返している）ので、素の JSON へ落としてから送ること。
   */
  it("reactive Proxy を素のオブジェクトへ落としてから送る（構造化複製できる形）", () => {
    const postMessage = vi.fn();
    const states = ref<Record<string, { memory?: Record<string, unknown> }>>({
      "report-1": { memory: { answered: true, rows: [{ id: 1 }] } },
    });
    const memory = states.value["report-1"].memory as Record<string, unknown>;
    // 前提の確認: Proxy をそのまま渡すと構造化複製できない
    expect(() => structuredClone(memory)).toThrow();

    postArtifactBridgeMemoryChanged(makeFrame({ postMessage }), memory);

    const [payload] = postMessage.mock.calls[0];
    expect(() => structuredClone(payload)).not.toThrow();
    expect(payload.memory).toEqual({ answered: true, rows: [{ id: 1 }] });

    // reactive() 由来でも同じ
    postMessage.mockClear();
    postArtifactBridgeMemoryChanged(makeFrame({ postMessage }), reactive({ a: 1 }));
    expect(() => structuredClone(postMessage.mock.calls[0][0])).not.toThrow();
  });

  it("iframe が消えていても例外を投げない", () => {
    expect(() => postArtifactBridgeMemoryChanged(null, { a: 1 })).not.toThrow();
    expect(() => postArtifactBridgeMemoryChanged(makeFrame(null), { a: 1 })).not.toThrow();
  });

  it("送れなかった場合は黙って捨てずに警告を残す", () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    try {
      const postMessage = vi.fn(() => {
        throw new Error("DataCloneError");
      });
      postArtifactBridgeMemoryChanged(makeFrame({ postMessage }), { a: 1 });
      expect(warn).toHaveBeenCalled();

      // シリアライズできない値も同様（循環参照）
      warn.mockClear();
      const circular: Record<string, unknown> = {};
      circular.self = circular;
      const ok = vi.fn();
      postArtifactBridgeMemoryChanged(makeFrame({ postMessage: ok }), circular);
      expect(warn).toHaveBeenCalled();
      expect(ok).not.toHaveBeenCalled();
    } finally {
      warn.mockRestore();
    }
  });
});

describe("buildReactSrcdoc のメモリー埋め込み", () => {
  it("メモリーを _memory textarea へ同期で読める形で埋める", () => {
    const html = buildReactSrcdoc("<head></head>", "export default () => null", undefined, {
      name: "taro",
    });
    expect(html).toContain('<textarea id="_memory" style="display:none">{&quot;name&quot;:&quot;taro&quot;}</textarea>');
  });

  it("メモリーが無ければ空オブジェクトを埋める", () => {
    const html = buildReactSrcdoc("<head></head>", "export default () => null");
    expect(html).toContain('<textarea id="_memory" style="display:none">{}</textarea>');
  });

  it("ブリッジは require('oretachi') を解決する RUNTIME より先に置く", () => {
    const html = buildReactSrcdoc("<head></head>", "export default () => null");
    expect(html.indexOf("window.__oretachi=")).toBeGreaterThan(-1);
    expect(html.indexOf("window.__oretachi=")).toBeLessThan(
      html.indexOf("libs['oretachi']=window.__oretachi"),
    );
  });
});

describe("srcdoc ごとの nonce", () => {
  const win = {};
  const frame = makeFrame(win);
  const base = {
    [ARTIFACT_BRIDGE_REQUEST_MARKER]: true,
    requestId: "r1",
    method: ARTIFACT_BRIDGE_METHOD_MEMORY_SET,
    params: {},
  };

  it("expectedNonce を渡すと、nonce が一致しないリクエスト（前の文書の遅延メッセージ）を捨てる", () => {
    expect(readArtifactBridgeRequest(makeEvent(win, { ...base, nonce: "new" }), frame, "new")).not.toBeNull();
    expect(readArtifactBridgeRequest(makeEvent(win, { ...base, nonce: "old" }), frame, "new")).toBeNull();
    expect(readArtifactBridgeRequest(makeEvent(win, base), frame, "new")).toBeNull();
  });

  it("expectedNonce を渡さない（通常のビューア）なら nonce を見ない", () => {
    expect(readArtifactBridgeRequest(makeEvent(win, base), frame)).not.toBeNull();
  });

  it("応答・通知・flush 要求に nonce を載せる", () => {
    const postMessage = vi.fn();
    const f = makeFrame({ postMessage });
    postArtifactBridgeResult(f, "r1", { ok: true }, "n1");
    postArtifactBridgeMemoryChanged(f, { a: 1 }, "n1");
    postArtifactBridgeFlush(f, "n1");
    expect(postMessage.mock.calls.map((c) => c[0].nonce)).toEqual(["n1", "n1", "n1"]);
    expect(postMessage.mock.calls[2][0].event).toBe(ARTIFACT_BRIDGE_PUSH_MEMORY_FLUSH);
    postArtifactBridgeResult(f, "r2", { ok: true });
    expect("nonce" in postMessage.mock.calls[3][0]).toBe(false);
  });
});

/** ブリッジ本体を、親ウィンドウと timer を差し替えた最小のグローバルで走らせる */
function runBridge(opts: { nonce?: string; memory?: string }) {
  const sent: Array<Record<string, any>> = [];
  const listeners: Array<(e: any) => void> = [];
  const timers: Array<() => void> = [];
  const parent = { postMessage: (m: Record<string, any>) => sent.push(m) };
  const win: Record<string, any> = {
    addEventListener: (_: string, fn: (e: any) => void) => listeners.push(fn),
  };
  if (opts.nonce !== undefined) win[ARTIFACT_BRIDGE_NONCE_GLOBAL] = opts.nonce;
  const document = { getElementById: () => ({ value: opts.memory ?? "{}" }) };
  const setTimeoutStub = (fn: () => void) => timers.push(fn);
  new Function("window", "parent", "document", "setTimeout", "clearTimeout", ARTIFACT_BRIDGE_JS)(
    win,
    parent,
    document,
    setTimeoutStub,
    () => {},
  );
  const deliver = (data: unknown) => listeners.forEach((fn) => fn({ source: parent, data }));
  return { win, sent, deliver, timers, oretachi: win.__oretachi };
}

describe("ブリッジ本体: nonce と flush", () => {
  it("nonce があるとき、リクエストに載せ、グローバルは消す", () => {
    const b = runBridge({ nonce: "n1" });
    expect(ARTIFACT_BRIDGE_NONCE_GLOBAL in b.win).toBe(false);
    void b.oretachi.callTool("notify_worktree", {}).catch(() => {});
    expect(b.sent[0].nonce).toBe("n1");
  });

  it("nonce が一致しない応答は捨てる（前の文書宛の応答が新しい待ちを解かない）", async () => {
    const b = runBridge({ nonce: "new" });
    const p = b.oretachi.callTool("notify_worktree", {});
    const id = b.sent[0].requestId;
    let settled = false;
    void p.then(() => (settled = true), () => (settled = true));
    b.deliver({ [ARTIFACT_BRIDGE_RESULT_MARKER]: true, requestId: id, nonce: "old", ok: true, result: "x" });
    await Promise.resolve();
    expect(settled).toBe(false);
    b.deliver({ [ARTIFACT_BRIDGE_RESULT_MARKER]: true, requestId: id, nonce: "new", ok: true, result: "x" });
    await expect(p).resolves.toBe("x");
  });

  it("flush 要求で debounce 待ちの保存を即送り、完了してから flushed を返す", async () => {
    const b = runBridge({ nonce: "n1" });
    const saved = b.oretachi.setMemoryKey("k", 1);
    // debounce 中: まだ保存は飛んでいない
    expect(b.sent.filter((m) => m.method === ARTIFACT_BRIDGE_METHOD_MEMORY_SET)).toHaveLength(0);
    b.deliver({ [ARTIFACT_BRIDGE_PUSH_MARKER]: true, event: ARTIFACT_BRIDGE_PUSH_MEMORY_FLUSH, nonce: "n1" });
    const set = b.sent.find((m) => m.method === ARTIFACT_BRIDGE_METHOD_MEMORY_SET)!;
    expect(set.params.memory).toEqual({ k: 1 });
    // 保存の応答が返るまで flushed は出ない
    expect(b.sent.some((m) => m.method === ARTIFACT_BRIDGE_METHOD_FLUSHED)).toBe(false);
    b.deliver({ [ARTIFACT_BRIDGE_RESULT_MARKER]: true, requestId: set.requestId, nonce: "n1", ok: true });
    await saved;
    const flushed = b.sent.find((m) => m.method === ARTIFACT_BRIDGE_METHOD_FLUSHED)!;
    expect(flushed.nonce).toBe("n1");
  });

  it("保存が無いときの flush 要求は、すぐ flushed を返す", () => {
    const b = runBridge({ nonce: "n1" });
    b.deliver({ [ARTIFACT_BRIDGE_PUSH_MARKER]: true, event: ARTIFACT_BRIDGE_PUSH_MEMORY_FLUSH, nonce: "n1" });
    expect(b.sent.filter((m) => m.method === ARTIFACT_BRIDGE_METHOD_FLUSHED)).toHaveLength(1);
  });

  it("nonce が一致しない flush 要求は無視する", () => {
    const b = runBridge({ nonce: "n1" });
    b.deliver({ [ARTIFACT_BRIDGE_PUSH_MARKER]: true, event: ARTIFACT_BRIDGE_PUSH_MEMORY_FLUSH, nonce: "other" });
    expect(b.sent).toHaveLength(0);
  });

  it("nonce の無い文書（通常のビューア）は従来どおり nonce を見ない", () => {
    const b = runBridge({});
    void b.oretachi.callTool("x", {}).catch(() => {});
    expect("nonce" in b.sent[0]).toBe(false);
  });
});

describe("ブリッジ本体: net.fetch", () => {
  it("引数の形を整えて net.fetch として中継する（headers はオブジェクトでも配列でもよい）", () => {
    const b = runBridge({ nonce: "n1" });
    void b.oretachi.net
      .fetch("https://api.example.com/v1/x", { method: "POST", headers: { A: "1", B: 2 }, body: "hi" })
      .catch(() => {});
    const m = b.sent[0];
    expect(m.method).toBe(ARTIFACT_BRIDGE_METHOD_NET_FETCH);
    expect(m.params).toMatchObject({
      url: "https://api.example.com/v1/x",
      method: "POST",
      headers: [["A", "1"], ["B", "2"]],
      body: "hi",
    });
    void b.oretachi.net.fetch("https://api.example.com/", { headers: [["X", "y"]] }).catch(() => {});
    expect(b.sent[1].params.headers).toEqual([["X", "y"]]);
    void b.oretachi.net.fetch("https://api.example.com/").catch(() => {});
    expect(b.sent[2].params.headers).toEqual([]);
  });

  it("URL・headers・body の型が不正なら、Rust へ送らず reject する", async () => {
    const b = runBridge({ nonce: "n1" });
    await expect(b.oretachi.net.fetch("")).rejects.toThrow(/URL/);
    await expect(b.oretachi.net.fetch(123)).rejects.toThrow(/URL/);
    await expect(b.oretachi.net.fetch("https://a.example.com/", { headers: "x" })).rejects.toThrow(/headers/);
    await expect(b.oretachi.net.fetch("https://a.example.com/", { headers: [null] })).rejects.toThrow(/headers/);
    await expect(b.oretachi.net.fetch("https://a.example.com/", { headers: ["ab"] })).rejects.toThrow(/headers/);
    await expect(b.oretachi.net.fetch("https://a.example.com/", { body: { a: 1 } })).rejects.toThrow(/body/);
    expect(b.sent).toHaveLength(0);
  });

  it("Rust の拒否は Error として reject される", async () => {
    const b = runBridge({ nonce: "n1" });
    const p = b.oretachi.net.fetch("https://evil.example.com/");
    b.deliver({
      [ARTIFACT_BRIDGE_RESULT_MARKER]: true,
      requestId: b.sent[0].requestId,
      nonce: "n1",
      ok: false,
      error: "承認された URL プレフィックスに含まれていません",
    });
    await expect(p).rejects.toThrow("承認された URL プレフィックス");
  });
});

