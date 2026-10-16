/**
 * アプリ（iframe）の `audio.*` ブリッジ呼び出しを、Rust コマンドへの呼び出しへ写す純関数。
 *
 * **ここでは権限の判断をしない。** 許可・上限・レートの検査は Rust（`app_audio.rs`）が行う唯一の関門で、
 * ここは「method 名 → コマンド名」と引数の形の写像だけ。
 */

import { ARTIFACT_BRIDGE_METHOD_AUDIO_PREFIX } from "./artifactMemory";

export interface AudioCommand {
  command: string;
  args: Record<string, unknown>;
}

/** `audio.*` 以外なら null。未知の `audio.*` は例外 */
export function audioCommandFor(method: string, params: Record<string, unknown>): AudioCommand | null {
  if (!method.startsWith(ARTIFACT_BRIDGE_METHOD_AUDIO_PREFIX)) return null;
  const name = method.slice(ARTIFACT_BRIDGE_METHOD_AUDIO_PREFIX.length);
  switch (name) {
    case "start": {
      const rate = params.sampleRate;
      // 数値以外は握りつぶさず、Rust の引数の型エラーにする（承認外のレートは Rust が拒否してログに残す）
      return { command: "app_audio_start", args: { sampleRate: rate === undefined || rate === null ? null : rate } };
    }
    case "stop":
      return { command: "app_audio_stop", args: {} };
    default:
      throw new Error(`audio.${name} は使えない API です`);
  }
}
