/** イベントトーストの表示責任（issue #130 / #137）。
 *
 *  Rust 側は `event-spawn-warning` を `app.emit`、つまり全 webview へブロードキャストする。
 *  素直に listen すると1件でメインとサブウィンドウが同時にトーストするので、
 *  「そのワークツリーを表示しているウィンドウ」だけが出すよう振り分ける。
 *
 *  #137 で配送トースト（`event-delivered`）は廃止したが、自動 spawn の警告は残るので
 *  この振り分けも残る。関数名の "Delivery" は当時の名残（呼び出し側は
 *  `useEventToast.ts` の1箇所ずつ）。
 *
 *  **1件=1トースト**がこの2関数の不変条件。純粋関数として切り出してテストで固定する。 */

/** メインウィンドウ。分離済み（サブウィンドウ持ち）のワークツリーは出さない。
 *
 *  `worktreeId` が null（バックエンドがワークツリーを解決できなかった）ときは**出す**。
 *  どのサブウィンドウにも紐づけられない以上、メインが出さなければ誰も出さない。 */
export function mainWindowShowsDelivery(
  worktreeId: string | null,
  isDetached: (id: string) => boolean,
): boolean {
  if (worktreeId === null) return true;
  return !isDetached(worktreeId);
}

/** サブウィンドウ。自分が担当するワークツリー宛だけ出す。
 *
 *  `worktreeId` が null のものはメインの担当なので出さない（両方が出すと重複する）。 */
export function subWindowShowsDelivery(
  worktreeId: string | null,
  ownWorktreeId: string,
): boolean {
  if (worktreeId === null) return false;
  return worktreeId === ownWorktreeId;
}
