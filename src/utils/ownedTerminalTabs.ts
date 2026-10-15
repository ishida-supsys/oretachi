/** インスタンス専用 AI ターミナル（#372）のタブ表示用のキー詰め替え。
 *
 *  所有情報（`useOwnedAiTerminals`）は PTY の `session_id` で引くが、`FramePane.vue` のタブは
 *  フロントの terminalId で引く。`terminalUnread.ts` と同じ逆引きを、メイン / サブ / トレイの
 *  3 窓で共有する。Vue の reactivity には触れない純粋関数なので `utils/`。 */

import type { SessionBearing } from "./terminalUnread";

/** タブのアイコン・ツールチップに出す情報 */
export interface OwnedTabInfo {
  /** 所有しているアプリ（インスタンス）の名前 */
  appName: string;
}

export function collectOwnedByTab(
  ownedBySession: Map<number, { appName: string }>,
  entries: Iterable<[number, SessionBearing | undefined | null]>,
  into: Map<number, OwnedTabInfo> = new Map(),
): Map<number, OwnedTabInfo> {
  // 所有端末が無いのが常態なので、空なら列を回さずに抜ける
  if (ownedBySession.size === 0) return into;
  for (const [terminalId, entry] of entries) {
    const sessionId = entry?.sessionId;
    if (sessionId == null) continue;
    const owner = ownedBySession.get(sessionId);
    if (owner) into.set(terminalId, { appName: owner.appName });
  }
  return into;
}
