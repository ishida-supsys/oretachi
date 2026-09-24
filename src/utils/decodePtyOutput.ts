/** `pty-output` の 1 セッション分。`data` は base64 文字列。 */
export interface PtyOutputChunk {
  sessionId: number;
  data: string;
}

/**
 * `pty-output` イベントの payload。Rust は 16ms 周期で出力のあった複数セッション分を
 * 1 回の emit に詰めて送る（セッション毎 emit は多端末時に UI スレッドを飽和させるため）。
 * 1 周期の合計が大きいときは複数回に分かれるが、同じセッションは 1 payload に高々 1 回。
 */
export interface PtyOutputBatchPayload {
  chunks: PtyOutputChunk[];
}

/**
 * Rust の `pty-output` の各チャンク（base64 文字列）を Uint8Array にデコードする。
 *
 * Rust 側は PTY 出力を `number[]`（Vec<u8>）ではなく base64 文字列として送る。
 * number[] のままだと巨大な eval 文字列になり WebView2 IPC を飽和させてハングの原因になるため。
 */
export function decodePtyOutput(data: string): Uint8Array {
  const binary = atob(data);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) {
    bytes[i] = binary.charCodeAt(i);
  }
  return bytes;
}
