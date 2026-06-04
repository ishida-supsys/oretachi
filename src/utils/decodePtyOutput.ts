/**
 * Rust の `pty-output` イベント payload（base64 文字列）を Uint8Array にデコードする。
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
