/**
 * アプリ（iframe）の `process.*` ブリッジ呼び出しを、Rust コマンドへの呼び出しへ写す純関数。
 *
 * **ここでは権限の判断をしない。** コマンドが宣言・承認されているか・引数の型・`under` の範囲・上限・停止は
 * Rust（`app_process.rs`）が行う唯一の関門で、ここは「method 名 → コマンド名」と引数の形の写像だけ。
 * 呼び出し元インスタンスは Rust が webview のウィンドウラベルから決めるので、ここでは宛先を指定させない
 * （`worktreeId` などが params に混ざっても渡さない）。
 */

import { ARTIFACT_BRIDGE_METHOD_PROCESS_PREFIX } from "./artifactMemory";

export interface ProcessCommand {
  command: string;
  args: Record<string, unknown>;
}

/** `process.*` 以外なら null。未知の `process.*` は例外 */
export function processCommandFor(method: string, params: Record<string, unknown>): ProcessCommand | null {
  if (!method.startsWith(ARTIFACT_BRIDGE_METHOD_PROCESS_PREFIX)) return null;
  const name = method.slice(ARTIFACT_BRIDGE_METHOD_PROCESS_PREFIX.length);
  // 型の合わない値は握りつぶさず素通しする（Rust の引数の型エラー・検査で拒否され、デバッグログに残る）
  switch (name) {
    case "start":
      return { command: "app_process_start", args: { name: params.name, params: params.params ?? null } };
    case "kill":
      return { command: "app_process_kill", args: { id: params.id } };
    case "list":
      return { command: "app_process_list", args: {} };
    case "read":
      return {
        command: "app_process_read",
        args: { id: params.id, sinceSeq: params.sinceSeq === undefined ? null : params.sinceSeq },
      };
    default:
      throw new Error(`process.${name} は使えない API です`);
  }
}
