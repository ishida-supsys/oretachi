/**
 * アプリ（iframe）の `net.ws.*` ブリッジ呼び出しを、Rust コマンドへの呼び出しへ写す純関数。
 *
 * **ここでは権限の判断をしない。** 接続先の許可・IP の検査・上限・切断は Rust（`app_ws.rs`）が行う唯一の関門で、
 * ここは「method 名 → コマンド名」と引数の形の写像だけ。
 */

import { ARTIFACT_BRIDGE_METHOD_NET_WS_PREFIX } from "./artifactMemory";

export interface WsCommand {
  command: string;
  args: Record<string, unknown>;
}

/** `net.ws.*` 以外なら null。未知の `net.ws.*` は例外 */
export function wsCommandFor(method: string, params: Record<string, unknown>): WsCommand | null {
  if (!method.startsWith(ARTIFACT_BRIDGE_METHOD_NET_WS_PREFIX)) return null;
  const name = method.slice(ARTIFACT_BRIDGE_METHOD_NET_WS_PREFIX.length);
  // 型の合わない値は握りつぶさず素通しする（Rust の引数の型エラー・検査で拒否され、デバッグログに残る）
  const id = params.id;
  switch (name) {
    case "open":
      return {
        command: "app_ws_open",
        args: {
          id,
          url: params.url,
          opts: { protocols: Array.isArray(params.protocols) ? params.protocols : [] },
        },
      };
    case "send":
      return { command: "app_ws_send", args: { id, data: params.data, binary: params.binary === true } };
    case "close":
      return {
        command: "app_ws_close",
        args: {
          id,
          code: params.code === undefined ? null : params.code,
          reason: params.reason === undefined ? null : params.reason,
        },
      };
    default:
      throw new Error(`net.ws.${name} は使えない API です`);
  }
}
