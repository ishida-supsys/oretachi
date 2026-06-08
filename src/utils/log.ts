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
//   - 設定の Debug Mode (settings.debugMode) を ON にすると本番でも詳細ログを復帰できる。
//     dev ビルドでは常に verbose (開発時の利便性のため)。
const isDev = import.meta.env.DEV;
let verboseOverride = false;

/**
 * 詳細ログ (debug/info) の override を設定する。
 * 設定タブの Debug Mode と連動させる想定。dev ビルドでは override に関わらず常に有効。
 */
export function setVerboseLogging(v: boolean): void {
  verboseOverride = v;
}

/** 現在の詳細ログ状態 (dev は常に true) */
export function isVerboseLogging(): boolean {
  return isDev || verboseOverride;
}

// fire-and-forget 送出。戻り値が Promise でない場合（テスト mock 等）でも壊れないよう
// Promise.resolve でラップしてから握り潰す。
function fire(p: unknown): void {
  void Promise.resolve(p).catch(() => {});
}

/** デバッグログ。本番(Debug Mode OFF)では IPC を発生させない。 */
export function logDebug(message: string): void {
  if (!isVerboseLogging()) return;
  fire(debug(message));
}

/** 情報ログ。本番(Debug Mode OFF)では IPC を発生させない。 */
export function logInfo(message: string): void {
  if (!isVerboseLogging()) return;
  fire(info(message));
}

/** 警告ログ。常に送るが await しない。 */
export function logWarn(message: string): void {
  fire(warn(message));
}

/** エラーログ。常に送るが await しない。 */
export function logError(message: string): void {
  fire(error(message));
}
