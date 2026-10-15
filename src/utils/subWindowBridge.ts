/**
 * アプリ（iframe）の `subWindow.*` ブリッジ呼び出しを、Rust コマンドへの呼び出しへ写す純関数。
 *
 * **ここでは権限の判断をしない。** 許可・上限・見た目（透明・枠なし・常に前面・位置）の検査は
 * Rust（`app_subwindow.rs`）が行う唯一の関門で、ここは「method 名 → コマンド名」と引数の形の写像だけ。
 */

import { ARTIFACT_BRIDGE_METHOD_SUB_WINDOW_PREFIX } from "./artifactMemory";

export interface SubWindowCommand {
  command: string;
  args: Record<string, unknown>;
}

const isObject = (v: unknown): v is Record<string, unknown> =>
  v !== null && typeof v === "object" && !Array.isArray(v);

function requireId(params: Record<string, unknown>): string {
  const id = params.id;
  if (typeof id !== "string" || id === "") throw new Error("id が必要です");
  return id;
}

function requireNumber(params: Record<string, unknown>, key: string): number {
  const v = params[key];
  if (typeof v !== "number" || !Number.isFinite(v)) throw new Error(`${key} は数値で指定してください`);
  return v;
}

/** `subWindow.*` 以外なら null。未知の `subWindow.*` は例外 */
export function subWindowCommandFor(
  method: string,
  params: Record<string, unknown>,
): SubWindowCommand | null {
  if (!method.startsWith(ARTIFACT_BRIDGE_METHOD_SUB_WINDOW_PREFIX)) return null;
  const name = method.slice(ARTIFACT_BRIDGE_METHOD_SUB_WINDOW_PREFIX.length);
  switch (name) {
    case "open":
      // 未知のキーの扱いも含め、検査は Rust 側。ここは素通しする
      return { command: "app_subwindow_open", args: { request: isObject(params) ? params : {} } };
    case "close":
      return { command: "app_subwindow_close", args: { id: requireId(params) } };
    case "list":
      return { command: "app_subwindow_list", args: {} };
    case "setPosition":
      return {
        command: "app_subwindow_set_position",
        args: { id: requireId(params), x: requireNumber(params, "x"), y: requireNumber(params, "y") },
      };
    case "setSize":
      return {
        command: "app_subwindow_set_size",
        args: { id: requireId(params), width: requireNumber(params, "width"), height: requireNumber(params, "height") },
      };
    case "startDrag":
      return { command: "app_subwindow_start_drag", args: {} };
    default:
      throw new Error(`サブウィンドウに無い API です: ${method}`);
  }
}
