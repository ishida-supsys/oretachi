import { debug, info, warn, error } from "@tauri-apps/plugin-log";

// ロギングゲート。
//
// webview 側の @tauri-apps/plugin-log の debug()/info()/warn()/error() は呼び出しごとに
// invoke('plugin:log|log') の IPC 往復を発生させる。この往復は Rust 側のレベルフィルタに
// 関係なく必ず発生し、ホットパスで多数発行すると WebView2 の単一 UI スレッドを飽和させ
// 恒久フリーズを引き起こす (issue #59, cf. 77475b4)。
//
// 対策:
//   - debug/info は本番ビルドでは呼び出し自体をスキップ (= IPC ゼロ)。
//   - warn/error は低頻度かつ重要なので常に送るが await しない (fire-and-forget)。
//   - 再現調査時は setVerboseLogging(true) で詳細ログを復帰可能。
let verbose = import.meta.env.DEV;

/** 詳細ログ (debug/info) の有効/無効を切り替える。再現調査用。 */
export function setVerboseLogging(v: boolean): void {
  verbose = v;
}

/** 現在の詳細ログ状態 */
export function isVerboseLogging(): boolean {
  return verbose;
}

/** デバッグログ。本番では IPC を発生させない。 */
export function logDebug(message: string): void {
  if (!verbose) return;
  void debug(message).catch(() => {});
}

/** 情報ログ。本番では IPC を発生させない。 */
export function logInfo(message: string): void {
  if (!verbose) return;
  void info(message).catch(() => {});
}

/** 警告ログ。常に送るが await しない。 */
export function logWarn(message: string): void {
  void warn(message).catch(() => {});
}

/** エラーログ。常に送るが await しない。 */
export function logError(message: string): void {
  void error(message).catch(() => {});
}
