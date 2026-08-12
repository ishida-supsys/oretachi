/** 配送トーストの表示責任（issue #130）。
 *
 *  Rust 側は `event-delivered` / `event-spawn-rejected` を `app.emit`、つまり全 webview へ
 *  ブロードキャストする。素直に listen すると1回の配送でメインとサブウィンドウが同時に
 *  トーストするので、「そのワークツリーを表示しているウィンドウ」だけが出すよう振り分ける。
 *
 *  **1配送=1トースト**がこの2関数の不変条件。純粋関数として切り出してテストで固定する。 */

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
