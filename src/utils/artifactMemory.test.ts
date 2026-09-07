import { describe, it, expect, vi } from "vitest";
import {
  ARTIFACT_BRIDGE_REQUEST_MARKER,
  ARTIFACT_BRIDGE_RESULT_MARKER,
  ARTIFACT_BRIDGE_METHOD_MEMORY_SET,
  readArtifactBridgeRequest,
  postArtifactBridgeResult,
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
