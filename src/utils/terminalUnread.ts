/** タブ未読バッジ用のキー詰め替え（issue #120 §7 / #130）。
 *
 *  バックエンド（`event_terminal_unread`）は PTY の `session_id` で未 ack 件数を数えるが、
 *  `FramePane.vue` のバッジは**フロントの terminalId**で引く。メイン / サブウィンドウ /
 *  トレイポップアップの3箇所が同じ逆引きループを持つことになるので、ここへ寄せる。
 *
 *  Vue の reactivity には触れない純粋関数なので `utils/`（`composables/` ではない）。 */

/** `sessionId` を持つ端末エントリの最小形。`TerminalView` インスタンス /
 *  `SubTerminalEntry` / `TrayTerminalEntry` のいずれもこの形に当てはまる。 */
export interface SessionBearing {
  sessionId?: number | null;
}

/** `session_id → 未 ack 件数` を `フロント terminalId → 未 ack 件数` へ詰め替える。
 *
 *  @param unreadBySession `useEventSubscriptions` の `terminalUnread`
 *  @param entries `[フロント terminalId, sessionId を持つ何か]` の列
 *  @param into 累積先。メインウィンドウのようにワークツリーごとの複数マップを
 *              1つへまとめたい場合に渡す
 */
export function collectUnreadByTab(
  unreadBySession: Map<number, number>,
  entries: Iterable<[number, SessionBearing | undefined | null]>,
  into: Map<number, number> = new Map(),
): Map<number, number> {
  // 未読ゼロが常態なので、空なら列を回さずに抜ける
  if (unreadBySession.size === 0) return into;
  for (const [terminalId, entry] of entries) {
    const sessionId = entry?.sessionId;
    if (sessionId == null) continue;
    const count = unreadBySession.get(sessionId);
    // 0 件は入れない（`FramePane` 側が `v-if="terminalUnread?.get(tid)"` の falsy 判定で出し分ける）
    if (count) into.set(terminalId, count);
  }
  return into;
}
