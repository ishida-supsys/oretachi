/**
 * 自動承認 ON のワークツリーにおける「AI 判定後に通知を出すか」の判定と、
 * AI 判定中に届いた `notify-worktree` の保留キュー（#168）。
 *
 * 自動承認 ON のワークツリーでは `useNotifications` 側の `shouldHold` が
 * approval/general を保留するため、通知を出せるのは AI 判定を回した後の経路だけになる。
 * その経路で通知が黙って消えないよう、判定中に届いたイベントをここで預かる。
 */

/** 判定中に預かった通知 1 ワークツリー分 */
export interface PendingNotify {
  /**
   * 預かったイベント群のうち 1 件でもトレイ表示対象（`tray !== false`）だったか。
   * `tray` は**イベント単位**の属性なので、後続の `tray: false` で上書きしない
   * （ワークツリー単位のラッチにすると直前の明示通知まで抑制されてしまう）。
   */
  tray: boolean;
}

export type PendingNotifyStore = Map<string, PendingNotify>;

export function createPendingNotifyStore(): PendingNotifyStore {
  return new Map<string, PendingNotify>();
}

/**
 * AI 判定中に届いたイベントを預かる。
 * `tray` は OR で畳む（明示通知が 1 件でもあれば提示する）。
 */
export function queuePendingNotify(store: PendingNotifyStore, worktreeId: string, tray: boolean): void {
  const existing = store.get(worktreeId);
  if (existing) {
    existing.tray = existing.tray || tray;
  } else {
    store.set(worktreeId, { tray });
  }
}

/** 預かっていた通知を取り出して消す。無ければ undefined */
export function takePendingNotify(store: PendingNotifyStore, worktreeId: string): PendingNotify | undefined {
  const pending = store.get(worktreeId);
  if (pending) store.delete(worktreeId);
  return pending;
}

/**
 * AI 判定を終えた（あるいは判定を回さず預かっていた）通知を提示するか。
 * - `approved`: AI が承認プロンプトを承認した = 人の操作は不要なので出さない
 * - `focused`: 既に見えているので出さない
 * - `tray`: イベント単位のトレイ表示可否（`notify-worktree` の `tray !== false`）
 */
export function shouldNotifyAfterJudge(params: {
  approved: boolean;
  focused: boolean;
  tray: boolean;
}): boolean {
  return !params.approved && !params.focused && params.tray;
}

/** `notify-worktree` ペイロードの `tray` を真偽値に正規化する（未指定は表示） */
export function trayOf(payload: { tray?: boolean }): boolean {
  return payload.tray !== false;
}
