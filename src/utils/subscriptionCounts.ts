import type { SubscriptionView } from "../types/event";

/** ワークツリーカードの購読バッジ（issue #137）の集計。
 *
 *  `event_list_subscriptions` が返す全件を1回読んでフロントで畳むだけで、ワークツリー
 *  ごとの新規コマンドは要らない。純粋関数として切り出してテストで固定する。 */

/** 1ワークツリーぶんの購読件数。 */
export interface SubscriptionCounts {
  /** ↑ このワークツリーのタブが張っている購読の件数（ワイルドカード購読もここに出る）。 */
  outgoing: number;
  /** ↓ このワークツリーを**厳密一致で**対象にしている購読の件数。 */
  incoming: number;
}

export const EMPTY_SUBSCRIPTION_COUNTS: Readonly<SubscriptionCounts> = Object.freeze({
  outgoing: 0,
  incoming: 0,
});

/** 厳密一致の target（単一ワークツリー指定）かどうか。
 *
 *  **`targetWorktreeName` が null であることを根拠にしないこと。** ワイルドカード購読は
 *  名前を解決できないので、それを条件にすると全部「クローズ済み」と誤判定される
 *  （#134 で実際に踏んで直した）。判定材料は `targetKind` だけ。 */
export function isExactWorktreeTarget(s: SubscriptionView): boolean {
  return s.targetKind === "worktree";
}

/** worktreeId → 購読件数。
 *
 *  - `outgoing` は購読者ワークツリーで数える。`*` / `workgroup:` / `repo:` の購読は
 *    「購読している側」の ↑ にだけ現れる
 *  - `incoming` は**厳密一致のみ**数える。`*` を1つ張られただけで全カードに ↓1 が付くと
 *    バッジの情報量が消えるため（#137 の決定事項）
 *  - `state === "orphaned"`（引き継ぎ待ち）も数える。除くと再起動直後に全部消えたように
 *    見える。区別はダイアログ側で見せる */
export function buildSubscriptionCounts(
  subs: SubscriptionView[],
): Map<string, SubscriptionCounts> {
  const counts = new Map<string, SubscriptionCounts>();
  const bump = (id: string, key: keyof SubscriptionCounts): void => {
    const cur = counts.get(id);
    if (cur) {
      cur[key] += 1;
      return;
    }
    counts.set(id, { outgoing: 0, incoming: 0, [key]: 1 });
  };

  for (const s of subs) {
    if (s.subscriberWorktreeId) bump(s.subscriberWorktreeId, "outgoing");
    if (isExactWorktreeTarget(s) && s.targetWorktreeId) bump(s.targetWorktreeId, "incoming");
  }
  return counts;
}
