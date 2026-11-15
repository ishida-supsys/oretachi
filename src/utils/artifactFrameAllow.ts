/**
 * アーティファクトの iframe に付ける `allow`（Permissions Policy の委譲）の決定（#438）。
 *
 * iframe は `sandbox="allow-scripts"`（allow-same-origin なし）で動くので、`navigator.clipboard.writeText`
 * は `allow="clipboard-write"` が無いと Permissions Policy で止まる。ここで許すのは iframe の中の
 * Web API だけで、Tauri の capability（`capabilities/appinst.json`）は増やさない。
 * 読み取り（`clipboard-read`）は許さない。
 */

/** 通常の React アーティファクトの iframe。承認の仕組みが無いので一律で許可する */
export const REACT_ARTIFACT_FRAME_ALLOW = "fullscreen; clipboard-write";

/**
 * アプリの iframe。`clipboardWrite` は Rust が承認済みの grant（`resolved.clipboard`）から決めた値で、
 * manifest や JS 側では判定しない。未承認は `undefined`（`allow` 属性ごと付けない）。
 */
export function appFrameAllow(clipboardWrite: boolean): string | undefined {
  return clipboardWrite ? "clipboard-write" : undefined;
}
